// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Disposable execution of one plugin-defined setup step.

#![allow(
    unsafe_code,
    reason = "the disposable setup child loads and invokes one ABI-checked native callback"
)]

use std::env;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::slice;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use libloading::Library;
use luminate_host_supervisor::sync_io::{read_frame, write_frame};
use luminate_plugin_api::{
    ABI_VERSION_SYMBOL_NAME, PLUGIN_ABI_VERSION, PLUGIN_DESCRIPTOR_SYMBOL_NAME,
    PLUGIN_SETUP_CBOR_CAPACITY, PluginAbiVersion, PluginDescriptor, PluginSetupRequest,
    PluginSetupStep, decode_cbor, encode_cbor,
};

const SETUP_MODE_ARGUMENT: &str = "--plugin-setup-step";
const SETUP_STEP_TIMEOUT: Duration = Duration::from_secs(35);

pub(crate) fn invocation_from_args() -> Result<Option<PathBuf>> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(OsStr::new(SETUP_MODE_ARGUMENT)) {
        return Ok(None);
    }
    let path = arguments
        .next()
        .context("--plugin-setup-step requires a plugin path")?;
    anyhow::ensure!(
        arguments.next().is_none(),
        "unexpected plugin setup argument"
    );
    Ok(Some(path.into()))
}

pub(crate) fn run(path: &Path) -> Result<()> {
    let request: PluginSetupRequest =
        read_frame(&mut io::stdin()).context("reading plugin setup request")?;
    let result = execute_loaded(path, &request).map_err(|error| format!("{error:#}"));
    write_frame(&mut io::stdout(), &result).context("writing plugin setup result")
}

pub(crate) fn execute(path: &Path, request: &PluginSetupRequest) -> Result<PluginSetupStep> {
    let executable = env::current_exe().context("locating luminated executable")?;
    let mut child = Command::new(executable)
        .arg(SETUP_MODE_ARGUMENT)
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning setup host for {}", path.display()))?;
    let mut input = child
        .stdin
        .take()
        .context("setup host stdin was not piped")?;
    write_frame(&mut input, request).context("writing plugin setup request")?;
    drop(input);
    let mut output = child
        .stdout
        .take()
        .context("setup host stdout was not piped")?;
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let reader = thread::Builder::new()
        .name("luminate-setup-result".to_owned())
        .spawn(move || {
            let result = read_frame(&mut output).context("reading plugin setup result");
            let _ = result_tx.send(result);
        })
        .context("spawning plugin setup result reader")?;
    let result: Result<PluginSetupStep, String> = match result_rx.recv_timeout(SETUP_STEP_TIMEOUT) {
        Ok(result) => result?,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            anyhow::bail!("plugin setup step exceeded its 35-second deadline");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            anyhow::bail!("plugin setup result reader stopped unexpectedly");
        }
    };
    let status = child.wait().context("waiting for plugin setup host")?;
    let _ = reader.join();
    anyhow::ensure!(status.success(), "plugin setup host exited with {status}");
    result.map_err(anyhow::Error::msg)
}

#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "the setup child performs one lifetime-bound load, ABI check, callback, and copy"
)]
fn execute_loaded(path: &Path, request: &PluginSetupRequest) -> Result<PluginSetupStep> {
    let payload = encode_cbor(request).map_err(anyhow::Error::msg)?;
    anyhow::ensure!(
        payload.len() <= PLUGIN_SETUP_CBOR_CAPACITY,
        "setup request is too large"
    );
    // SAFETY: the library remains loaded until the callback response has been
    // copied and decoded, and its exact ABI version is checked first.
    unsafe {
        let library =
            Library::new(path).with_context(|| format!("failed to open {}", path.display()))?;
        let abi_version = library
            .get::<*const PluginAbiVersion>(ABI_VERSION_SYMBOL_NAME)
            .with_context(|| format!("missing ABI version symbol in {}", path.display()))?;
        anyhow::ensure!(
            **abi_version == PLUGIN_ABI_VERSION,
            "plugin ABI mismatch: plugin={}, daemon={PLUGIN_ABI_VERSION}",
            **abi_version
        );
        let descriptor = &**library
            .get::<*const PluginDescriptor>(PLUGIN_DESCRIPTOR_SYMBOL_NAME)
            .with_context(|| format!("missing plugin descriptor symbol in {}", path.display()))?;
        let callback = descriptor
            .setup_cbor
            .context("plugin does not expose a setup callback")?;
        let mut length = 0;
        let pointer = callback(payload.as_ptr(), payload.len(), &raw mut length);
        anyhow::ensure!(
            length <= PLUGIN_SETUP_CBOR_CAPACITY,
            "plugin setup response is too large"
        );
        anyhow::ensure!(!pointer.is_null(), "plugin setup response pointer is null");
        anyhow::ensure!(length != 0, "plugin setup response is empty");
        let bytes = slice::from_raw_parts(pointer, length).to_vec();
        let result: Result<PluginSetupStep, String> =
            decode_cbor(&bytes).map_err(anyhow::Error::msg)?;
        result.map_err(anyhow::Error::msg)
    }
}

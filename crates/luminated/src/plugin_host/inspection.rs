// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Side-effect-free plugin metadata inspection in a disposable child process.

#![allow(
    unsafe_code,
    reason = "native plugin symbols are opened only in the disposable inspection child"
)]

use std::collections::HashSet;
use std::env;
use std::ffi::{CStr, OsStr, c_char};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::slice;

use anyhow::{Context as _, Result};
use libloading::Library;
use luminate_host_supervisor::sync_io::{read_frame, write_frame};
use luminate_plugin_api::{
    ABI_VERSION_SYMBOL_NAME, PLUGIN_ABI_VERSION, PLUGIN_DESCRIPTOR_SYMBOL_NAME, PluginAbiVersion,
    PluginBus, PluginDescriptor, PluginSettingApplyMode, PluginSettingDescriptor,
    PluginSettingKind, PluginSetupWorkflowDescriptor,
};
use luminate_protocol::{PluginSetupWorkflow, PluginSetupWorkflowKind};
use serde::{Deserialize, Serialize};

const INSPECT_MODE_ARGUMENT: &str = "--plugin-inspect";
const MAX_METADATA_ITEMS: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InspectionResult {
    name: String,
    version: String,
    buses: Vec<u32>,
    settings: Vec<InspectedSetting>,
    setup_workflows: Vec<InspectedSetupWorkflow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InspectedSetupWorkflow {
    id: String,
    label: String,
    description: String,
    kind: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InspectedSetting {
    key: String,
    label: String,
    description: String,
    kind: u32,
    default_toml: Option<String>,
    required: bool,
    sensitive: bool,
    apply_mode: u32,
    minimum: Option<f64>,
    maximum: Option<f64>,
    constraints_toml: Option<String>,
}

/// Validated static metadata needed during activation selection.
#[derive(Debug, Clone)]
pub(crate) struct InspectedPlugin {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) buses: Vec<PluginBus>,
    pub(crate) settings: Vec<InspectedPluginSetting>,
    pub(crate) setup_workflows: Vec<PluginSetupWorkflow>,
}

/// One validated setting from an inspected plugin's static schema.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InspectedPluginSetting {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) kind: PluginSettingKind,
    pub(crate) default: Option<toml::Value>,
    pub(crate) required: bool,
    pub(crate) sensitive: bool,
    pub(crate) apply_mode: PluginSettingApplyMode,
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
    pub(crate) constraints: Option<toml::Value>,
}

pub(crate) fn invocation_from_args() -> Result<Option<PathBuf>> {
    let mut arguments = env::args_os().skip(1);
    if arguments.next().as_deref() != Some(OsStr::new(INSPECT_MODE_ARGUMENT)) {
        return Ok(None);
    }
    let path = arguments
        .next()
        .context("--plugin-inspect requires a plugin path")?;
    anyhow::ensure!(
        arguments.next().is_none(),
        "unexpected plugin inspection argument"
    );
    Ok(Some(path.into()))
}

pub(crate) fn run(path: &Path) -> Result<()> {
    let result = inspect_loaded_image(path).map_err(|error| format!("{error:#}"));
    write_frame(&mut io::stdout(), &result).context("writing plugin inspection result")
}

pub(crate) fn inspect(path: &Path) -> Result<InspectedPlugin> {
    let executable = env::current_exe().context("locating luminated executable")?;
    let mut child = Command::new(executable)
        .arg(INSPECT_MODE_ARGUMENT)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning plugin inspector for {}", path.display()))?;
    let mut output = child
        .stdout
        .take()
        .context("plugin inspector stdout was not piped")?;
    let result: Result<InspectionResult, String> =
        read_frame(&mut output).context("reading plugin inspection result")?;
    let status = child.wait().context("waiting for plugin inspector")?;
    anyhow::ensure!(status.success(), "plugin inspector exited with {status}");
    let metadata = result.map_err(anyhow::Error::msg)?;
    let buses = metadata
        .buses
        .into_iter()
        .map(|bus| PluginBus::from_abi(bus).context("plugin declared an unknown bus"))
        .collect::<Result<Vec<_>>>()?;
    let settings = metadata
        .settings
        .into_iter()
        .map(InspectedPluginSetting::try_from)
        .collect::<Result<Vec<_>>>()?;
    let setup_workflows = metadata
        .setup_workflows
        .into_iter()
        .map(|workflow| {
            let kind = match workflow.kind {
                0 => PluginSetupWorkflowKind::Provision,
                1 => PluginSetupWorkflowKind::Repair,
                2 => PluginSetupWorkflowKind::Discover,
                3 => PluginSetupWorkflowKind::Import,
                4 => PluginSetupWorkflowKind::FactoryProvision,
                value => anyhow::bail!("plugin declared unknown setup workflow kind {value}"),
            };
            Ok(PluginSetupWorkflow::new(
                metadata.name.clone(),
                workflow.id,
                workflow.label,
                workflow.description,
                kind,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(InspectedPlugin {
        name: metadata.name,
        version: metadata.version,
        buses,
        settings,
        setup_workflows,
    })
}

impl TryFrom<InspectedSetting> for InspectedPluginSetting {
    type Error = anyhow::Error;

    fn try_from(setting: InspectedSetting) -> Result<Self> {
        let kind = match setting.kind {
            value if value == PluginSettingKind::Boolean as u32 => PluginSettingKind::Boolean,
            value if value == PluginSettingKind::Integer as u32 => PluginSettingKind::Integer,
            value if value == PluginSettingKind::Number as u32 => PluginSettingKind::Number,
            value if value == PluginSettingKind::String as u32 => PluginSettingKind::String,
            value if value == PluginSettingKind::Enumeration as u32 => {
                PluginSettingKind::Enumeration
            }
            value if value == PluginSettingKind::Array as u32 => PluginSettingKind::Array,
            value => anyhow::bail!("plugin declared unknown setting kind {value}"),
        };
        let apply_mode = match setting.apply_mode {
            value if value == PluginSettingApplyMode::RestartRequired as u32 => {
                PluginSettingApplyMode::RestartRequired
            }
            value => anyhow::bail!("plugin declared unknown setting apply mode {value}"),
        };
        let default = setting
            .default_toml
            .map(|value| {
                value
                    .parse()
                    .context("plugin setting default is not a TOML value")
            })
            .transpose()?;
        let constraints = setting
            .constraints_toml
            .map(|value| {
                value
                    .parse()
                    .context("plugin setting constraints are not a TOML value")
            })
            .transpose()?;

        Ok(Self {
            key: setting.key,
            label: setting.label,
            description: setting.description,
            kind,
            default,
            required: setting.required,
            sensitive: setting.sensitive,
            apply_mode,
            minimum: setting.minimum,
            maximum: setting.maximum,
            constraints,
        })
    }
}

#[allow(
    clippy::multiple_unsafe_ops_per_block,
    reason = "the disposable inspection child opens and reads one ABI-checked static descriptor as one lifetime-bound operation"
)]
fn inspect_loaded_image(path: &Path) -> Result<InspectionResult> {
    // SAFETY: this process exists only to inspect one image and exits
    // immediately. ABI compatibility is checked before the descriptor is read.
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
        let name = read_required_string(descriptor.name, "name")?;
        anyhow::ensure!(name.is_ascii(), "plugin name must be ASCII");
        anyhow::ensure!(!name.trim().is_empty(), "plugin name must not be empty");
        let version = read_required_string(descriptor.version, "version")?;
        let buses = read_raw_codes(descriptor.buses.cast(), descriptor.bus_count, "buses")?;
        for bus in &buses {
            anyhow::ensure!(
                PluginBus::from_abi(*bus).is_some(),
                "plugin declared unknown bus code {bus}"
            );
        }
        let raw_settings = read_slice(
            descriptor.settings,
            descriptor.setting_count,
            "settings schema",
        )?;
        let mut settings = Vec::with_capacity(raw_settings.len());
        let mut keys = HashSet::new();
        for setting in raw_settings {
            let inspected = inspect_setting(setting)?;
            anyhow::ensure!(
                keys.insert(inspected.key.clone()),
                "duplicate plugin setting key {}",
                inspected.key
            );
            settings.push(inspected);
        }
        let raw_workflows = read_slice(
            descriptor.setup_workflows,
            descriptor.setup_workflow_count,
            "setup workflows",
        )?;
        anyhow::ensure!(
            raw_workflows.is_empty() == descriptor.setup_cbor.is_none(),
            "plugin setup workflow metadata and callback must be present together"
        );
        let mut setup_workflows = Vec::with_capacity(raw_workflows.len());
        let mut workflow_ids = HashSet::new();
        for workflow in raw_workflows {
            let inspected = inspect_setup_workflow(workflow)?;
            anyhow::ensure!(
                workflow_ids.insert(inspected.id.clone()),
                "duplicate plugin setup workflow ID {}",
                inspected.id
            );
            setup_workflows.push(inspected);
        }
        drop(library);
        Ok(InspectionResult {
            name,
            version,
            buses,
            settings,
            setup_workflows,
        })
    }
}

fn inspect_setup_workflow(
    workflow: &PluginSetupWorkflowDescriptor,
) -> Result<InspectedSetupWorkflow> {
    let id = read_required_string(workflow.id, "setup workflow ID")?;
    anyhow::ensure!(id.len() <= 128, "setup workflow ID is too long");
    anyhow::ensure!(valid_dotted_key(&id), "invalid setup workflow ID {id:?}");
    anyhow::ensure!(
        luminate_plugin_api::PluginSetupWorkflowKind::from_abi(workflow.kind).is_some(),
        "unknown setup workflow kind {}",
        workflow.kind
    );
    let label = read_required_string(workflow.label, "setup workflow label")?;
    let description = read_required_string(workflow.description, "setup workflow description")?;
    anyhow::ensure!(
        !label.trim().is_empty() && label.len() <= 256 && !label.chars().any(char::is_control),
        "setup workflow label is invalid"
    );
    anyhow::ensure!(
        !description.trim().is_empty()
            && description.len() <= 2_048
            && !description.chars().any(char::is_control),
        "setup workflow description is invalid"
    );
    Ok(InspectedSetupWorkflow {
        id,
        label,
        description,
        kind: workflow.kind,
    })
}

fn inspect_setting(setting: &PluginSettingDescriptor) -> Result<InspectedSetting> {
    let key = read_required_string(setting.key, "setting key")?;
    anyhow::ensure!(valid_dotted_key(&key), "invalid plugin setting key {key:?}");
    let valid_kind = [
        PluginSettingKind::Boolean,
        PluginSettingKind::Integer,
        PluginSettingKind::Number,
        PluginSettingKind::String,
        PluginSettingKind::Enumeration,
        PluginSettingKind::Array,
    ]
    .iter()
    .any(|kind| *kind as u32 == setting.kind);
    anyhow::ensure!(valid_kind, "unknown plugin setting kind {}", setting.kind);
    anyhow::ensure!(
        setting.apply_mode == PluginSettingApplyMode::RestartRequired as u32,
        "unknown plugin setting apply mode {}",
        setting.apply_mode
    );
    let default_toml = read_optional_string(setting.default_toml)?;
    if let Some(default_toml) = &default_toml {
        let _: toml::Value = default_toml
            .parse()
            .context("plugin setting default is not a TOML value")?;
    }
    let constraints_toml = read_optional_string(setting.constraints)?;
    if let Some(constraints_toml) = &constraints_toml {
        let _: toml::Value = constraints_toml
            .parse()
            .context("plugin setting constraints are not a TOML value")?;
    }
    let minimum = setting.has_minimum.then_some(setting.minimum);
    let maximum = setting.has_maximum.then_some(setting.maximum);
    if let (Some(minimum), Some(maximum)) = (minimum, maximum) {
        anyhow::ensure!(
            minimum <= maximum,
            "plugin setting minimum exceeds its maximum"
        );
    }
    Ok(InspectedSetting {
        key,
        label: read_required_string(setting.label, "setting label")?,
        description: read_required_string(setting.description, "setting description")?,
        kind: setting.kind,
        default_toml,
        required: setting.required,
        sensitive: setting.sensitive,
        apply_mode: setting.apply_mode,
        minimum,
        maximum,
        constraints_toml,
    })
}

fn valid_dotted_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
}

fn read_raw_codes(pointer: *const u32, length: usize, label: &str) -> Result<Vec<u32>> {
    Ok(read_slice(pointer, length, label)?.to_vec())
}

fn read_slice<'a, T>(pointer: *const T, length: usize, label: &str) -> Result<&'a [T]> {
    anyhow::ensure!(length <= MAX_METADATA_ITEMS, "{label} has too many entries");
    if length == 0 {
        return Ok(&[]);
    }
    anyhow::ensure!(!pointer.is_null(), "{label} pointer is null");
    // SAFETY: the plugin ABI requires `length` static elements; the count is
    // bounded and the pointer was checked above.
    Ok(unsafe { slice::from_raw_parts(pointer, length) })
}

fn read_required_string(pointer: *const c_char, label: &str) -> Result<String> {
    anyhow::ensure!(!pointer.is_null(), "{label} pointer is null");
    // SAFETY: the plugin ABI requires static NUL-terminated strings.
    let value = unsafe { CStr::from_ptr(pointer) };
    Ok(value
        .to_str()
        .with_context(|| format!("{label} is not UTF-8"))?
        .to_owned())
}

fn read_optional_string(pointer: *const c_char) -> Result<Option<String>> {
    if pointer.is_null() {
        return Ok(None);
    }
    read_required_string(pointer, "optional setting metadata").map(Some)
}

#[cfg(test)]
#[path = "inspection_tests.rs"]
mod tests;

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Live validation for Windows named-pipe client identity capture.
//!
//! Binds a named pipe, connects a client to it in the same process, and
//! impersonates that client to read back its real PID and SID via
//! [`luminate_platform::windows::identity`]. This proves the capture path
//! works against an actual Windows security token, not just that the code
//! compiles. Deliberately minimal: no connection pooling or request dispatch.
//!
//! Only meaningful on Windows; compiles to a no-op elsewhere so
//! `cargo build --workspace` still succeeds on Linux and macOS.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "This runnable validation example intentionally reports its captured identity and \
              platform-support status to the console."
)]

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::process::ExitCode;

#[cfg(windows)]
#[tokio::main]
async fn main() -> ExitCode {
    match windows_impl::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                luminate_platform::terminal::escape(&format!("error: {error}"))
            );
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("windows_named_pipe_identity only runs on Windows; skipping.");
}

#[cfg(windows)]
mod windows_impl {
    use std::io;
    use std::os::windows::io::AsRawHandle as _;

    use luminate_platform::windows::identity::{
        captured_client_sid, current_process_sid, named_pipe_client_process_id,
    };
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    const PIPE_NAME: &str = r"\\.\pipe\luminate-identity-validation";

    pub async fn run() -> io::Result<()> {
        let server = ServerOptions::new().create(PIPE_NAME)?;
        let connected = server.connect();

        // The pipe instance already exists once `create` returns, so the
        // client can open it immediately; `connected` resolves once the
        // server side observes that connection complete.
        let client_open = tokio::spawn(async { ClientOptions::new().open(PIPE_NAME) });

        connected.await?;
        let client = client_open.await.map_err(io::Error::other)??;

        let handle = server.as_raw_handle();
        let pid = named_pipe_client_process_id(handle)?;
        let sid = captured_client_sid(handle)?;
        let daemon_sid = current_process_sid()?;

        println!("captured client identity: pid={pid} sid={sid}");
        println!("daemon's own sid:         {daemon_sid}");
        assert_eq!(
            sid, daemon_sid,
            "client and server are the same process here, so their SIDs must match"
        );
        println!(
            "OK: captured SID matches the daemon's own SID, as expected for a same-process client."
        );

        drop(client);
        Ok(())
    }
}

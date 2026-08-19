// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Connects to Luminate and prints an authoritative topology snapshot.

#![allow(
    clippy::print_stdout,
    clippy::use_debug,
    reason = "This runnable inspection example intentionally prints structured target values."
)]

use std::env;
use std::io::{self, Write as _};
use std::process::ExitCode;

use luminate::{Client, TargetId};
use luminate_platform::terminal::escape;

const SOCKET_PATH_ENV: &str = "LUMINATED_SOCKET_PATH";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(
                io::stderr().lock(),
                "{}",
                escape(&format!("error: {error}"))
            );
            ExitCode::FAILURE
        }
    }
}

async fn run() -> luminate::Result<()> {
    let client = match env::var_os(SOCKET_PATH_ENV) {
        Some(path) => Client::connect_path(path).await?,
        None => Client::connect().await?,
    };
    let server = client.server_info().await?;
    println!(
        "{} {} (protocol ABI {})",
        escape(&server.daemon_name),
        escape(&server.daemon_version),
        server.protocol_abi_version
    );

    for device in client.list_devices().await? {
        println!("{}: {}", escape(device.id.as_str()), escape(&device.name));

        // Constructors preserve target structure without parsing or joining IDs.
        let device_target = TargetId::device(device.id.as_str());
        println!("  target: {}", escape(&format!("{device_target:?}")));
        for surface in &device.surfaces {
            let surface_target = TargetId::surface(device.id.as_str(), surface.id.as_str());
            println!(
                "  surface {}: {}",
                escape(&surface.name),
                escape(&format!("{surface_target:?}"))
            );
            for element in &surface.elements {
                let element_target =
                    TargetId::element(device.id.as_str(), surface.id.as_str(), element.id.as_str());
                println!(
                    "    element {}: {}",
                    escape(element.id.as_str()),
                    escape(&format!("{element_target:?}"))
                );
            }
        }
        for group in &device.groups {
            let group_target = TargetId::group(device.id.as_str(), group.id.as_str());
            println!(
                "  group {}: {}",
                escape(&group.name),
                escape(&format!("{group_target:?}"))
            );
        }
    }

    Ok(())
}

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Command-line client for inspecting and controlling Luminate targets.

mod cli;
mod commands;
mod management_output;
mod output;

use std::io::{self, Write as _};
use std::process::ExitCode;

use luminate_platform::terminal::escape;

#[tokio::main]
async fn main() -> ExitCode {
    match commands::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let clap_error = error.downcast_ref::<clap::Error>();
            let exit_code = clap_error.map_or(1, clap::Error::exit_code);
            let message = clap_error.map_or_else(
                || format!("error: {error:#}"),
                |error| error.render().ansi().to_string(),
            );
            if clap_error.is_some() {
                let _ = write!(io::stderr().lock(), "{message}");
            } else {
                let _ = writeln!(io::stderr().lock(), "{}", escape(&message));
            }
            u8::try_from(exit_code).map_or(ExitCode::FAILURE, ExitCode::from)
        }
    }
}

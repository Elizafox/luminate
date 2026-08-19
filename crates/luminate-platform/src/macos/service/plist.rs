// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Rendering the `launchd` property list for the `luminated` `LaunchDaemon`.
//!
//! Hand-written XML rather than a `plist`-crate dependency: every value here
//! is a known-safe path, account name, or boolean fixed by this module, with
//! no untrusted input to escape. The embedded template keeps the plist easy to
//! review without duplicating it in Rust source.

use std::path::Path;

use super::account::{SERVICE_GROUP, SERVICE_USER};

/// The `launchd` label. Must match the plist's own filename
/// (`{LABEL}.plist`); `launchd` requires that correspondence and otherwise
/// silently misbehaves.
pub const LABEL: &str = "com.wilcoxti.luminate.luminated";

const TEMPLATE: &str = include_str!("com.wilcoxti.luminate.luminated.plist.in");

/// Renders the property list content for a `luminated` binary at
/// `program_path`, logging to `log_path`.
#[must_use]
pub fn render(program_path: &Path, log_path: &Path) -> String {
    TEMPLATE
        .replace("@LABEL@", LABEL)
        .replace("@SERVICE_USER@", SERVICE_USER)
        .replace("@SERVICE_GROUP@", SERVICE_GROUP)
        .replace("@PROGRAM_PATH@", &program_path.display().to_string())
        .replace("@LOG_PATH@", &log_path.display().to_string())
}

#[cfg(test)]
#[path = "plist_tests.rs"]
mod tests;

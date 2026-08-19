// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Isolated native-plugin hosting across a supervised child-process boundary.

mod inspection;
mod protocol;
mod runtime;
mod setup;
mod shm;
mod supervisor;
#[cfg(test)]
#[path = "tests/fixture.rs"]
mod test_support;
mod validation;

pub(crate) use inspection::{
    InspectedPlugin, InspectedPluginSetting, inspect,
    invocation_from_args as inspection_invocation_from_args, run as run_inspection,
};
pub use protocol::{ApplyOutcome, BeginShmStreamRequest, ShmStreamOutcome};
pub use runtime::{invocation_from_args, run};
pub(crate) use setup::{
    execute as execute_setup_step, invocation_from_args as setup_invocation_from_args,
    run as run_setup_step,
};
pub use supervisor::{HostedPlugin, current_max_plugin_log_level};

#[cfg(test)]
pub(crate) use runtime::{RawProbeHint, ensure_abi_compatible, read_bus_slice, read_probe_hints};

#[cfg(test)]
pub(crate) use test_support::lock_fixture;

const HOST_MODE_ARGUMENT: &str = "--plugin-host";
const HOST_LOG_LEVEL_ENV: &str = "LUMINATE_PLUGIN_HOST_LOG_LEVEL";

pub(crate) const MAX_PLUGIN_BATCH_UPDATES: usize = 64;
pub(crate) const MAX_PLUGIN_READ_TARGETS: usize = 256;

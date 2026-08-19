// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use std::path::PathBuf;

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn unix_defaults_use_the_existing_fhs_layout() {
    assert_eq!(
        super::config().unwrap(),
        PathBuf::from("/etc/luminate/luminated.toml")
    );
    assert_eq!(
        super::socket().unwrap(),
        PathBuf::from("/run/luminated.sock")
    );
    assert_eq!(
        super::state().unwrap(),
        PathBuf::from("/var/lib/luminated/state.json")
    );
    assert_eq!(
        super::http_state_dir().unwrap(),
        PathBuf::from("/var/lib/luminate-http")
    );
    assert_eq!(
        super::plugin_local().unwrap(),
        PathBuf::from("/usr/local/lib/luminate/plugins")
    );
    assert_eq!(
        super::plugin_system().unwrap(),
        PathBuf::from("/usr/lib/luminate/plugins")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_defaults_use_the_system_daemon_layout() {
    assert_eq!(
        super::config().unwrap(),
        PathBuf::from("/Library/Application Support/Luminate/luminated.toml")
    );
    assert_eq!(
        super::socket().unwrap(),
        PathBuf::from("/var/run/luminated/luminated.sock")
    );
    assert_eq!(
        super::state().unwrap(),
        PathBuf::from("/Library/Application Support/Luminate/state/state.json")
    );
    assert_eq!(
        super::http_state_dir().unwrap(),
        PathBuf::from("/Library/Application Support/Luminate/http")
    );
    assert_eq!(
        super::plugin_local().unwrap(),
        PathBuf::from("/usr/local/lib/luminate/plugins")
    );
    assert_eq!(
        super::plugin_system().unwrap(),
        PathBuf::from("/Library/Application Support/Luminate/plugins")
    );
}

#[test]
fn compiled_defaults_are_available_and_event_path_is_derived() {
    for path in [
        super::default_config_path().expect("default config path"),
        super::default_socket_path().expect("default socket path"),
        super::default_state_path().expect("default state path"),
        super::default_http_state_dir().expect("default HTTP state directory"),
        super::default_plugin_dir_local().expect("default local plugin path"),
        super::default_plugin_dir_system().expect("default system plugin path"),
    ] {
        assert!(!path.as_os_str().is_empty());
    }
    assert_eq!(
        super::event_socket_path("/run/luminated.sock"),
        PathBuf::from("/run/luminated.sock.events")
    );
}

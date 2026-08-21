// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

#[cfg(unix)]
use crate::authorization::RateLimitKey;

use super::*;

#[test]
fn parses_a_partial_config_against_defaults() {
    let config: DaemonConfig = toml::from_str("prefer_shm = false\n").unwrap();
    assert!(!config.prefer_shm);
    // Unspecified fields still come from `Default`.
    assert_eq!(config.socket_path, DaemonConfig::default().socket_path);
    assert_eq!(config.default_unsupported_policy, UnsupportedPolicy::Skip);
}

#[test]
fn parses_default_unsupported_policy() {
    let config: DaemonConfig = toml::from_str("default_unsupported_policy = \"Reject\"\n").unwrap();
    assert_eq!(config.default_unsupported_policy, UnsupportedPolicy::Reject);
}

#[test]
fn parses_reconciliation_policy_overrides() {
    let config: DaemonConfig = toml::from_str(
        r#"
reconciliation_policy = "Leave"
[device_reconciliation]
shared_bulb = "Adopt"

[[plugins]]
name = "luminate-plugin-lifx"
reconciliation = "Adopt"
"#,
    )
    .expect("parse reconciliation policies");
    assert_eq!(
        config.reconciliation_policy,
        Some(ReconciliationPolicy::Leave)
    );
    assert_eq!(
        config
            .device_reconciliation
            .get(&DeviceId::new("shared_bulb")),
        Some(&ReconciliationPolicy::Adopt)
    );
    assert_eq!(
        config.plugins[0].reconciliation,
        Some(ReconciliationPolicy::Adopt)
    );
}

#[test]
fn rejects_unknown_top_level_keys() {
    // A security-relevant typo must fail loudly rather than being silently
    // ignored and leaving the activation policy at its default.
    let error = toml::from_str::<DaemonConfig>("autlaod = false\n").unwrap_err();
    assert!(
        error.to_string().contains("autlaod"),
        "error should name the unknown key, got: {error}"
    );
}

#[test]
fn rejects_removed_autoload_setting() {
    let error = toml::from_str::<DaemonConfig>("autoload = false\n").unwrap_err();
    assert!(
        error.to_string().contains("autoload"),
        "error should name the removed key, got: {error}"
    );
}

#[test]
fn rejects_unknown_plugin_keys() {
    let error = toml::from_str::<DaemonConfig>("[[plugins]]\nname = \"demo\"\nrequried = true\n")
        .unwrap_err();
    assert!(
        error.to_string().contains("requried"),
        "error should name the unknown plugin key, got: {error}"
    );
}

#[test]
fn rejects_removed_per_plugin_grab_bag_sections() {
    let reconciliation =
        toml::from_str::<DaemonConfig>("[plugin_reconciliation]\nexample = \"Adopt\"\n")
            .unwrap_err();
    assert!(reconciliation.to_string().contains("plugin_reconciliation"));

    let management =
        toml::from_str::<DaemonConfig>("[[plugin_management.plugins]]\nname = \"example\"\n")
            .unwrap_err();
    assert!(management.to_string().contains("plugins"));
}

#[test]
fn validate_rejects_empty_socket_path() {
    let config = DaemonConfig {
        socket_path: PathBuf::new(),
        ..DaemonConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn validate_rejects_empty_state_path() {
    let config = DaemonConfig {
        state_path: PathBuf::new(),
        ..DaemonConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn validate_rejects_matching_event_and_primary_socket_paths() {
    let path = PathBuf::from("/tmp/luminated.sock");
    let config = DaemonConfig {
        socket_path: path.clone(),
        event_socket_path: Some(path),
        ..DaemonConfig::default()
    };
    assert!(config.validate().is_err());
}

#[test]
fn validate_rejects_plugin_without_name_or_path() {
    let mut config = DaemonConfig::default();
    config.plugins.push(PluginConfig::default());
    let error = config.validate().unwrap_err();
    assert!(
        error.to_string().contains("plugins[0]"),
        "error should point at the offending entry, got: {error}"
    );
}

#[test]
fn validate_accepts_a_plugin_with_only_a_name() {
    let mut config = DaemonConfig::default();
    config.plugins.push(PluginConfig {
        name: Some("demo".to_owned()),
        ..PluginConfig::default()
    });
    assert!(config.validate().is_ok());
}

#[test]
fn parses_nested_plugin_configuration() {
    let config: DaemonConfig = toml::from_str(
        r#"
[[plugins]]
name = "luminate-plugin-wled"
[plugins.config]
mdns = false
endpoints = ["192.0.2.10", "wled.example:8080"]
"#,
    )
    .expect("parse nested plugin configuration");
    let plugin = config.plugins.first().expect("configured plugin");
    assert_eq!(
        plugin.config.get("mdns"),
        Some(&toml::Value::Boolean(false))
    );
    assert_eq!(
        plugin
            .config
            .get("endpoints")
            .and_then(toml::Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn absent_authorization_section_defaults_to_socket_access() {
    let config: DaemonConfig = toml::from_str("").unwrap();
    assert_eq!(config.authorization.policy, PolicyKind::SocketAccess);
    assert_eq!(
        config.authorization.pipe_access,
        PipeAccessConfig::LocalGroup {
            group: "Luminate Clients".to_owned(),
        }
    );
}

#[test]
fn parses_windows_pipe_access_principals() {
    let local_group: DaemonConfig = toml::from_str(
            "[authorization.pipe_access]\nprincipal = \"local-group\"\ngroup = \"Lighting Operators\"\n",
        )
        .unwrap();
    assert_eq!(
        local_group.authorization.pipe_access,
        PipeAccessConfig::LocalGroup {
            group: "Lighting Operators".to_owned(),
        }
    );

    let interactive: DaemonConfig =
        toml::from_str("[authorization.pipe_access]\nprincipal = \"interactive\"\n").unwrap();
    assert_eq!(
        interactive.authorization.pipe_access,
        PipeAccessConfig::Interactive {}
    );
}

#[test]
fn rejects_invalid_windows_pipe_access() {
    let unknown = toml::from_str::<DaemonConfig>(
        "[authorization.pipe_access]\nprincipal = \"authenticated-users\"\n",
    )
    .unwrap_err();
    assert!(unknown.to_string().contains("unknown variant"));

    let empty_group: DaemonConfig = toml::from_str(
        "[authorization.pipe_access]\nprincipal = \"local-group\"\ngroup = \"  \"\n",
    )
    .unwrap();
    let error = empty_group.validate().unwrap_err();
    assert_eq!(
        error.to_string(),
        "authorization.pipe_access.group must not be empty"
    );

    let irrelevant_group = toml::from_str::<DaemonConfig>(
        "[authorization.pipe_access]\nprincipal = \"interactive\"\ngroup = \"ignored\"\n",
    )
    .unwrap_err();
    assert!(irrelevant_group.to_string().contains("unknown field"));
}

#[test]
fn parses_explicit_socket_access_policy() {
    let config: DaemonConfig =
        toml::from_str("[authorization]\npolicy = \"socket-access\"\n").unwrap();
    assert_eq!(config.authorization.policy, PolicyKind::SocketAccess);
}

#[test]
fn rejects_unknown_authorization_policy() {
    // An unrecognized policy name must fail startup rather than silently
    // falling back to the permissive built-in default.
    let error =
        toml::from_str::<DaemonConfig>("[authorization]\npolicy = \"allow-all\"\n").unwrap_err();
    assert!(
        error.to_string().contains("allow-all"),
        "error should name the rejected policy, got: {error}"
    );
}

#[test]
fn rejects_unknown_authorization_keys() {
    let error = toml::from_str::<DaemonConfig>("[authorization]\nprovider = \"x\"\n").unwrap_err();
    assert!(
        error.to_string().contains("provider"),
        "error should name the unknown key, got: {error}"
    );
}

#[test]
fn parses_and_validates_authorization_limits_and_registrations() {
    let config: DaemonConfig = toml::from_str(
        r#"
[authorization.limits]
connections = 8
subscriptions = 4
[[authorization.frontends]]
platform = "unix"
uid = 1001
[[authorization.frontends]]
platform = "unix"
user = "luminate-http"
[[authorization.frontends]]
platform = "windows"
account = "NT SERVICE\\luminate-dbus"
"#,
    )
    .expect("parse authorization configuration");
    config
        .validate()
        .expect("valid authorization configuration");
    assert_eq!(config.authorization.limits.connections, Some(8));
    assert_eq!(config.authorization.frontends.len(), 3);

    let invalid: DaemonConfig =
        toml::from_str("[authorization.limits]\nconnections = 0\n").expect("parse zero limit");
    assert!(invalid.validate().is_err());

    assert!(
        toml::from_str::<DaemonConfig>("[authorization.limits]\nconcurrent_requests = 2\n")
            .is_err()
    );
}

#[test]
fn rejects_empty_registered_windows_frontend_identity() {
    let config: DaemonConfig = toml::from_str(
        r#"
[[authorization.frontends]]
platform = "windows"
sid = "  "
"#,
    )
    .expect("parse frontend registration");
    assert!(config.validate().is_err());
}

#[test]
fn rejects_ambiguous_or_missing_frontend_account_references() {
    for registration in [
        "platform = \"unix\"",
        "platform = \"unix\"\nuid = 1001\nuser = \"luminate-http\"",
        "platform = \"windows\"",
        "platform = \"windows\"\nsid = \"S-1-5-18\"\naccount = \"SYSTEM\"",
    ] {
        let input = format!("[[authorization.frontends]]\n{registration}\n");
        let config: DaemonConfig = toml::from_str(&input).expect("parse registration shape");
        assert!(config.validate().is_err(), "accepted {registration}");
    }
}

#[cfg(unix)]
#[test]
fn resolves_unix_frontend_ids_and_rejects_missing_accounts() {
    let numeric: DaemonConfig =
        toml::from_str("[[authorization.frontends]]\nplatform = \"unix\"\nuid = 4242\n")
            .expect("parse numeric registration");
    assert_eq!(
        numeric.authorization.frontends[0]
            .resolve()
            .expect("resolve numeric registration"),
        Some(RateLimitKey::Uid(4242))
    );

    let missing: DaemonConfig = toml::from_str(
        "[[authorization.frontends]]\nplatform = \"unix\"\nuser = \
         \"luminate-account-that-must-not-exist\"\n",
    )
    .expect("parse named registration");
    assert!(missing.authorization.frontends[0].resolve().is_err());
}

#[test]
fn validates_managed_daemon_setting_locks() {
    let valid: DaemonConfig = toml::from_str(
        r#"
[management]
locked_daemon_settings = [
    "prefer_shm",
    "device_reconciliation.desk",
]
"#,
    )
    .expect("parse valid daemon locks");
    valid.validate().expect("validate daemon locks");

    let unknown: DaemonConfig =
        toml::from_str("[management]\nlocked_daemon_settings = [\"socket_path\"]\n")
            .expect("parse unknown daemon lock");
    assert!(unknown.validate().is_err());

    let duplicate: DaemonConfig =
        toml::from_str("[management]\nlocked_daemon_settings = [\"prefer_shm\", \"prefer_shm\"]\n")
            .expect("parse duplicate daemon lock");
    assert!(duplicate.validate().is_err());
}

#[test]
fn validates_plugin_setting_locks() {
    let invalid: DaemonConfig = toml::from_str(
        r#"
[plugin_management]
[[plugins]]
name = "luminate-plugin-lifx"
locked_settings = ["connection..address"]
"#,
    )
    .expect("parse invalid plugin setting lock");
    assert!(invalid.validate().is_err());
}

#[test]
fn plugin_management_configuration_uses_host_attached_by_default() {
    let default: DaemonConfig = toml::from_str("").expect("parse default configuration");
    assert_eq!(
        default.plugin_activation_mode(),
        PluginActivationMode::HostAttached
    );

    let managed: DaemonConfig = toml::from_str(
        r#"
[management]
managed_config_path = "/var/lib/luminated/operator.toml"
locked_daemon_settings = ["prefer_shm"]

[plugin_management]
activation = "host-attached"

[[plugins]]
name = "luminate-plugin-lifx"
activation = "disabled"
locked_settings = ["discovery_address"]
"#,
    )
    .expect("parse managed plugin configuration");
    managed.validate().expect("validate managed configuration");

    assert_eq!(
        managed.plugin_activation_mode(),
        PluginActivationMode::HostAttached
    );
    assert_eq!(
        managed.managed_config_path().expect("managed config path"),
        Path::new("/var/lib/luminated/operator.toml")
    );
    assert_eq!(
        managed.plugins[0].activation,
        ManagedPluginActivation::Disabled
    );
    assert_eq!(managed.plugins[0].locked_settings, ["discovery_address"]);
}

#[test]
fn load_errors_on_a_missing_file() {
    let error = DaemonConfig::load(Path::new("/does/not/exist/luminated.toml")).unwrap_err();
    assert!(error.to_string().contains("failed to read config file"));
}

#[test]
fn validates_external_authentication_provider_configuration() {
    let valid_toml = if cfg!(windows) {
        r#"
[[authorization.authentication_providers]]
name = "company-sso"
authority = "company.example"
executable = 'C:\Program Files\Luminate\luminate-auth-company.exe'
initialization_file = 'C:\ProgramData\Luminate\company-sso.secret'
max_sessions = 12
"#
    } else {
        r#"
[[authorization.authentication_providers]]
name = "company-sso"
authority = "company.example"
executable = "/usr/libexec/luminate-auth-company"
initialization_file = "/etc/luminate/company-sso.secret"
max_sessions = 12
"#
    };
    let valid: DaemonConfig = toml::from_str(valid_toml).expect("parse authentication provider");
    valid.validate().expect("validate authentication provider");
    let provider = &valid.authorization.authentication_providers[0];
    assert_eq!(provider.name, "company-sso");
    assert_eq!(provider.authority, "company.example");
    assert!(provider.required);
    assert_eq!(provider.max_sessions, 12);

    for invalid in [
        r#"
[[authorization.authentication_providers]]
name = "bad name"
authority = "example"
executable = "/provider"
initialization_file = "/secret"
"#,
        r#"
[[authorization.authentication_providers]]
name = "duplicate"
authority = "one"
executable = "/provider"
initialization_file = "/secret"
[[authorization.authentication_providers]]
name = "duplicate"
authority = "two"
executable = "/provider-two"
initialization_file = "/secret-two"
"#,
        r#"
[[authorization.authentication_providers]]
name = "relative"
authority = "example"
executable = "provider"
initialization_file = "/secret"
"#,
    ] {
        let config: DaemonConfig = toml::from_str(invalid).expect("parse invalid provider");
        assert!(config.validate().is_err());
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

fn arguments<'a>(values: &'a [&'a str]) -> impl Iterator<Item = OsString> + 'a {
    values.iter().map(OsString::from)
}

#[test]
fn ignores_normal_daemon_invocation() {
    assert_eq!(parse(arguments(&[])).unwrap(), None);
}

#[test]
fn parses_status() {
    assert_eq!(
        parse(arguments(&["service", "status"])).unwrap(),
        Some(Command::Status)
    );
}

#[test]
fn parses_install_and_explicit_development_override() {
    assert_eq!(
        parse(arguments(&["service", "install"])).unwrap(),
        Some(Command::Install {
            allow_development_path: false,
            add_users: Vec::new(),
            add_current_user: false,
            start_on_install: false,
        })
    );
    assert_eq!(
        parse(arguments(&[
            "service",
            "install",
            "--allow-development-path"
        ]))
        .unwrap(),
        Some(Command::Install {
            allow_development_path: true,
            add_users: Vec::new(),
            add_current_user: false,
            start_on_install: false,
        })
    );
}

#[test]
fn parses_install_start_on_install_flag() {
    assert_eq!(
        parse(arguments(&["service", "install", "--start"])).unwrap(),
        Some(Command::Install {
            allow_development_path: false,
            add_users: Vec::new(),
            add_current_user: false,
            start_on_install: true,
        })
    );
}

#[test]
fn parses_install_membership_options_in_any_order() {
    assert_eq!(
        parse(arguments(&[
            "service",
            "install",
            "--add-user",
            r"DOMAIN\Ada",
            "--allow-development-path",
            "--add-current-user",
            "--add-user",
            r"MACHINE\Grace",
        ]))
        .unwrap(),
        Some(Command::Install {
            allow_development_path: true,
            add_users: vec![r"DOMAIN\Ada".to_owned(), r"MACHINE\Grace".to_owned()],
            add_current_user: true,
            start_on_install: false,
        })
    );
}

#[test]
fn rejects_add_user_without_account() {
    assert_eq!(
        parse(arguments(&["service", "install", "--add-user"]))
            .unwrap_err()
            .to_string(),
        "--add-user requires an account name"
    );
}

#[test]
fn parses_start_and_stop() {
    assert_eq!(
        parse(arguments(&["service", "start"])).unwrap(),
        Some(Command::Start)
    );
    assert_eq!(
        parse(arguments(&["service", "stop"])).unwrap(),
        Some(Command::Stop)
    );
}

#[test]
fn parses_uninstall() {
    assert_eq!(
        parse(arguments(&["service", "uninstall"])).unwrap(),
        Some(Command::Uninstall {
            purge_client_group: false
        })
    );
    assert_eq!(
        parse(arguments(&["service", "uninstall", "--purge-client-group"])).unwrap(),
        Some(Command::Uninstall {
            purge_client_group: true
        })
    );
}

#[test]
fn rejects_missing_command() {
    assert_eq!(
        parse(arguments(&["service"])).unwrap_err().to_string(),
        "service command required"
    );
}

#[test]
fn rejects_unknown_command() {
    assert_eq!(
        parse(arguments(&["service", "dance"]))
            .unwrap_err()
            .to_string(),
        "unknown service command: dance"
    );
}

#[test]
fn rejects_trailing_arguments() {
    assert_eq!(
        parse(arguments(&["service", "status", "extra"]))
            .unwrap_err()
            .to_string(),
        "unexpected argument to service status: extra"
    );
}

#[test]
fn trailing_argument_names_the_selected_command() {
    assert_eq!(
        parse(arguments(&["service", "stop", "now"]))
            .unwrap_err()
            .to_string(),
        "unexpected argument to service stop: now"
    );
}

#[test]
fn macos_ignores_normal_daemon_invocation() {
    assert_eq!(parse_macos(arguments(&[])).unwrap(), None);
}

#[test]
fn macos_parses_status() {
    assert_eq!(
        parse_macos(arguments(&["service", "status"])).unwrap(),
        Some(MacosCommand::Status)
    );
}

#[test]
fn macos_parses_install_and_explicit_development_override() {
    assert_eq!(
        parse_macos(arguments(&["service", "install"])).unwrap(),
        Some(MacosCommand::Install {
            allow_development_path: false,
            start_on_install: false,
        })
    );
    assert_eq!(
        parse_macos(arguments(&[
            "service",
            "install",
            "--allow-development-path",
            "--start"
        ]))
        .unwrap(),
        Some(MacosCommand::Install {
            allow_development_path: true,
            start_on_install: true,
        })
    );
}

#[test]
fn macos_parses_start_and_stop() {
    assert_eq!(
        parse_macos(arguments(&["service", "start"])).unwrap(),
        Some(MacosCommand::Start)
    );
    assert_eq!(
        parse_macos(arguments(&["service", "stop"])).unwrap(),
        Some(MacosCommand::Stop)
    );
}

#[test]
fn macos_parses_uninstall() {
    assert_eq!(
        parse_macos(arguments(&["service", "uninstall"])).unwrap(),
        Some(MacosCommand::Uninstall {
            purge_account: false
        })
    );
    assert_eq!(
        parse_macos(arguments(&["service", "uninstall", "--purge-account"])).unwrap(),
        Some(MacosCommand::Uninstall {
            purge_account: true
        })
    );
}

#[test]
fn macos_rejects_missing_command() {
    assert_eq!(
        parse_macos(arguments(&["service"]))
            .unwrap_err()
            .to_string(),
        "service command required"
    );
}

#[test]
fn macos_rejects_unknown_command() {
    assert_eq!(
        parse_macos(arguments(&["service", "dance"]))
            .unwrap_err()
            .to_string(),
        "unknown service command: dance"
    );
}

#[test]
fn macos_rejects_trailing_arguments() {
    assert_eq!(
        parse_macos(arguments(&["service", "status", "extra"]))
            .unwrap_err()
            .to_string(),
        "unexpected argument to service status: extra"
    );
}

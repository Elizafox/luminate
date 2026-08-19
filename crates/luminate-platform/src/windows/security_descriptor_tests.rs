// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;

#[test]
fn owner_only_descriptor_has_one_full_access_ace() {
    assert_eq!(
        owner_only_sddl("S-1-5-21-1234"),
        "D:P(A;;FA;;;S-1-5-21-1234)"
    );
}

#[test]
fn service_pipe_descriptor_has_canonical_ace_order_and_restricted_client_access() {
    assert_eq!(SERVICE_PIPE_CLIENT_ACCESS & FILE_CREATE_PIPE_INSTANCE, 0);
    assert_eq!(
        service_pipe_sddl("S-1-5-80-1234", "S-1-5-21-5678"),
        "D:P(A;;FA;;;S-1-5-80-1234)(A;;FA;;;BA)(A;;0x12019b;;;S-1-5-21-5678)"
    );
}

#[test]
fn machine_data_root_descriptor_admits_only_system_and_administrators() {
    assert_eq!(
        machine_data_root_sddl(),
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    );
}

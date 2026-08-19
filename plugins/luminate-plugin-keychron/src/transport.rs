// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! QMK Raw HID request/response transport.

use hidapi::HidDevice;

pub(crate) const REPORT_LENGTH: usize = 32;
const REPORT_ID: u8 = 0;
const RESPONSE_TIMEOUT_MS: i32 = 250;

pub(crate) trait Transport {
    fn transact(&self, request: [u8; REPORT_LENGTH]) -> Result<[u8; REPORT_LENGTH], String>;
}

pub(crate) struct HidTransport {
    device: HidDevice,
}

impl HidTransport {
    pub(crate) const fn new(device: HidDevice) -> Self {
        Self { device }
    }
}

impl Transport for HidTransport {
    fn transact(&self, request: [u8; REPORT_LENGTH]) -> Result<[u8; REPORT_LENGTH], String> {
        let mut write_buffer = [0; REPORT_LENGTH + 1];
        write_buffer[0] = REPORT_ID;
        write_buffer[1..].copy_from_slice(&request);
        let written = self
            .device
            .write(&write_buffer)
            .map_err(|error| format!("failed to write Keychron Raw HID report: {error}"))?;
        if written != write_buffer.len() {
            return Err(format!(
                "Keychron Raw HID write accepted {written} bytes, expected {}",
                write_buffer.len()
            ));
        }

        let mut response = [0; REPORT_LENGTH];
        let received = self
            .device
            .read_timeout(&mut response, RESPONSE_TIMEOUT_MS)
            .map_err(|error| format!("failed to read Keychron Raw HID report: {error}"))?;
        if received != REPORT_LENGTH {
            return Err(format!(
                "Keychron Raw HID response has {received} bytes, expected {REPORT_LENGTH}"
            ));
        }

        Ok(response)
    }
}

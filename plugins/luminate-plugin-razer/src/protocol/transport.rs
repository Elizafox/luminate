// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! HID feature-report transport for the common Razer protocol.

use std::error::Error;
use std::ffi::CStr;
use std::fmt;
use std::thread;
use std::time::Duration;

use hidapi::{HidApi, HidDevice};

use super::commands::{Command, ResponsePolicy};
use super::report::{REPORT_LEN, Report, ReportError};
use super::response::{self, ResponseError};

const HID_REPORT_ID: u8 = 0;
const HID_BUFFER_LEN: usize = REPORT_LEN + 1;
const MAX_BUSY_ATTEMPTS: usize = 3;
const BUSY_RETRY_DELAY: Duration = Duration::from_millis(10);

pub(crate) trait Channel {
    fn send_feature_report(&self, bytes: &[u8]) -> Result<(), String>;
    fn get_feature_report(&self, bytes: &mut [u8]) -> Result<usize, String>;
}

impl Channel for HidDevice {
    fn send_feature_report(&self, bytes: &[u8]) -> Result<(), String> {
        HidDevice::send_feature_report(self, bytes)
            .map_err(|error| format!("failed to send Razer feature report: {error}"))
    }

    fn get_feature_report(&self, bytes: &mut [u8]) -> Result<usize, String> {
        HidDevice::get_feature_report(self, bytes)
            .map_err(|error| format!("failed to receive Razer feature report: {error}"))
    }
}

pub(crate) trait DeviceOpener {
    fn open_path(&self, path: &CStr) -> Result<Box<dyn Channel>, String>;
}

pub(crate) struct HidApiOpener;

impl DeviceOpener for HidApiOpener {
    fn open_path(&self, path: &CStr) -> Result<Box<dyn Channel>, String> {
        let api = HidApi::new().map_err(|error| format!("failed to initialize hidapi: {error}"))?;
        let device = api
            .open_path(path)
            .map_err(|error| format!("failed to open selected Razer HID path: {error}"))?;

        Ok(Box::new(device))
    }
}

pub(crate) fn execute(
    channel: &dyn Channel,
    command: &Command,
    response_delay: Duration,
) -> Result<Option<Vec<u8>>, TransportError> {
    for attempt in 1..=MAX_BUSY_ATTEMPTS {
        match execute_once(channel, command, response_delay) {
            Err(TransportError::Response(ResponseError::Busy))
                if command.idempotent() && attempt < MAX_BUSY_ATTEMPTS =>
            {
                thread::sleep(BUSY_RETRY_DELAY);
            }
            result => return result,
        }
    }

    Err(TransportError::Response(ResponseError::Busy))
}

fn execute_once(
    channel: &dyn Channel,
    command: &Command,
    response_delay: Duration,
) -> Result<Option<Vec<u8>>, TransportError> {
    let mut request = [0; HID_BUFFER_LEN];
    request[0] = HID_REPORT_ID;
    request[1..].copy_from_slice(&command.report().encode());
    channel
        .send_feature_report(&request)
        .map_err(TransportError::Io)?;
    thread::sleep(response_delay);

    if command.response_policy() == ResponsePolicy::None {
        return Ok(None);
    }

    let mut response_buffer = [0; HID_BUFFER_LEN];
    response_buffer[0] = HID_REPORT_ID;
    let received = channel
        .get_feature_report(&mut response_buffer)
        .map_err(TransportError::Io)?;
    if received != HID_BUFFER_LEN {
        return Err(TransportError::InvalidFeatureReportLength(received));
    }
    if response_buffer[0] != HID_REPORT_ID {
        return Err(TransportError::UnexpectedReportId(response_buffer[0]));
    }

    let response = Report::decode(&response_buffer[1..]).map_err(TransportError::Report)?;
    let arguments = response::validate(command.report(), &response)
        .map_err(TransportError::Response)?
        .to_vec();
    Ok(Some(arguments))
}

#[derive(Debug)]
pub(crate) enum TransportError {
    Io(String),
    InvalidFeatureReportLength(usize),
    UnexpectedReportId(u8),
    Report(ReportError),
    Response(ResponseError),
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => formatter.write_str(message),
            Self::InvalidFeatureReportLength(length) => write!(
                formatter,
                "Razer HID feature response has {length} bytes, expected {HID_BUFFER_LEN}"
            ),
            Self::UnexpectedReportId(id) => {
                write!(
                    formatter,
                    "Razer HID response has unexpected report ID {id:#04x}"
                )
            }
            Self::Report(error) => write!(formatter, "invalid Razer response report: {error}"),
            Self::Response(error) => error.fmt(formatter),
        }
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Report(error) => Some(error),
            Self::Response(error) => Some(error),
            Self::Io(_) | Self::InvalidFeatureReportLength(_) | Self::UnexpectedReportId(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::env;
    use std::time::Instant;

    use super::*;
    use crate::protocol::commands::{
        ColourMode, Direction, LedTarget, MatrixEffect, ReactiveSpeed, Rgb, StarlightSpeed, Storage,
    };

    const TEST_VENDOR_ID: u16 = 0x1532;
    const TEST_PRODUCT_ID: u16 = 0x028d;
    const TEST_CONTROL_INTERFACE: i32 = 3;
    const TEST_TRANSACTION_ID: u8 = 0x1f;
    const TEST_RESPONSE_DELAY: Duration = Duration::from_micros(600);

    struct LiveKeyboard {
        channel: Box<dyn Channel>,
    }

    impl LiveKeyboard {
        fn open() -> Self {
            Self {
                channel: open_live_keyboard(&HidApiOpener)
                    .expect("authorized test keyboard control interface should open"),
            }
        }

        fn reconnect_after_physical_disconnect() -> Self {
            const POLL_INTERVAL: Duration = Duration::from_millis(100);
            const TRANSITION_TIMEOUT: Duration = Duration::from_secs(60);

            let opener = HidApiOpener;
            let transition_started = Instant::now();
            while open_live_keyboard(&opener).is_ok() {
                assert!(
                    transition_started.elapsed() < TRANSITION_TIMEOUT,
                    "keyboard was not physically disconnected within 60 seconds"
                );
                thread::sleep(POLL_INTERVAL);
            }

            loop {
                if let Ok(channel) = open_live_keyboard(&opener) {
                    return Self { channel };
                }
                assert!(
                    transition_started.elapsed() < TRANSITION_TIMEOUT,
                    "keyboard did not reconnect within 60 seconds"
                );
                thread::sleep(POLL_INTERVAL);
            }
        }

        fn restore_spectrum(&self) {
            execute(
                self.as_ref(),
                &Command::extended_matrix_effect(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    MatrixEffect::Spectrum,
                )
                .expect("spectrum effect should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("spectrum restoration should succeed");
        }
    }

    fn open_live_keyboard(opener: &impl DeviceOpener) -> Result<Box<dyn Channel>, String> {
        let api = HidApi::new().map_err(|error| error.to_string())?;
        let info = api
            .device_list()
            .find(|device| {
                device.vendor_id() == TEST_VENDOR_ID
                    && device.product_id() == TEST_PRODUCT_ID
                    && device.interface_number() == TEST_CONTROL_INTERFACE
            })
            .ok_or_else(|| "test keyboard is unavailable".to_owned())?;
        opener.open_path(info.path())
    }

    impl AsRef<dyn Channel> for LiveKeyboard {
        fn as_ref(&self) -> &(dyn Channel + 'static) {
            self.channel.as_ref()
        }
    }

    struct FakeChannel {
        sent: RefCell<Vec<Vec<u8>>>,
        responses: RefCell<VecDeque<Vec<u8>>>,
    }

    impl Channel for FakeChannel {
        fn send_feature_report(&self, bytes: &[u8]) -> Result<(), String> {
            self.sent.borrow_mut().push(bytes.to_vec());
            Ok(())
        }

        fn get_feature_report(&self, bytes: &mut [u8]) -> Result<usize, String> {
            let response = self
                .responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| "fake response queue is empty".to_owned())?;
            bytes.copy_from_slice(&response);
            Ok(response.len())
        }
    }

    fn successful_response(command: &Command, arguments: &[u8]) -> Vec<u8> {
        let mut report = command.report().encode();
        report[0] = 2;
        report[8..8 + arguments.len()].copy_from_slice(arguments);
        report[88] = report[2..88].iter().fold(0, |crc, byte| crc ^ byte);
        let mut feature = vec![0];
        feature.extend_from_slice(&report);
        feature
    }

    #[test]
    fn executes_feature_query_with_leading_zero_report_id() {
        let command = Command::firmware_version(0x1f).expect("valid command");
        let channel = FakeChannel {
            sent: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::from([successful_response(&command, &[1, 2])])),
        };

        let response = execute(&channel, &command, Duration::ZERO)
            .expect("query should succeed")
            .expect("query should return data");

        assert_eq!(response, [1, 2]);
        assert_eq!(channel.sent.borrow().len(), 1);
        assert_eq!(channel.sent.borrow()[0].len(), HID_BUFFER_LEN);
        assert_eq!(channel.sent.borrow()[0][0], HID_REPORT_ID);
    }

    #[test]
    fn retries_busy_idempotent_command_with_a_strict_bound() {
        let command = Command::firmware_version(0x1f).expect("valid command");
        let mut busy = successful_response(&command, &[0, 0]);
        busy[1] = 1;
        busy[89] = busy[3..89].iter().fold(0, |crc, byte| crc ^ byte);
        let channel = FakeChannel {
            sent: RefCell::new(Vec::new()),
            responses: RefCell::new(VecDeque::from([
                busy,
                successful_response(&command, &[1, 0]),
            ])),
        };

        let response = execute(&channel, &command, Duration::ZERO)
            .expect("busy response should be retried")
            .expect("query should return data");

        assert_eq!(response, [1, 0]);
        assert_eq!(channel.sent.borrow().len(), 2);
    }

    #[test]
    #[ignore = "requires the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    fn queries_attached_blackwidow_v4_pro_identity() {
        let channel = LiveKeyboard::open();
        let serial = execute(
            channel.as_ref(),
            &Command::serial(TEST_TRANSACTION_ID).expect("serial command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("serial query should succeed")
        .expect("serial query should return a response");
        let repeated_serial = execute(
            channel.as_ref(),
            &Command::serial(TEST_TRANSACTION_ID).expect("serial command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("repeated serial query should succeed")
        .expect("repeated serial query should return a response");
        let firmware = execute(
            channel.as_ref(),
            &Command::firmware_version(TEST_TRANSACTION_ID)
                .expect("firmware command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("firmware query should succeed")
        .expect("firmware query should return a response");

        assert!(
            serial.iter().any(|byte| *byte != 0),
            "serial must not be empty"
        );
        assert_eq!(
            serial, repeated_serial,
            "serial must be stable within a session"
        );
        assert_eq!(firmware, [1, 0]);
    }

    #[test]
    #[ignore = "changes brightness on the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    fn validates_attached_blackwidow_v4_pro_brightness_endpoints() {
        let channel = LiveKeyboard::open();
        let original = execute(
            channel.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness query should succeed")
        .expect("brightness query should return data")
        .get(2)
        .copied()
        .expect("brightness response should contain a value");

        for brightness in [0, 255] {
            execute(
                channel.as_ref(),
                &Command::set_brightness(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    brightness,
                )
                .expect("brightness command should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("brightness write should succeed");
            let observed = execute(
                channel.as_ref(),
                &Command::get_brightness(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                )
                .expect("brightness query should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("brightness query should succeed")
            .expect("brightness query should return data");
            assert_eq!(observed.get(2), Some(&brightness));
        }

        execute(
            channel.as_ref(),
            &Command::set_brightness(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                original,
            )
            .expect("brightness restoration command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness restoration should succeed");
    }

    #[test]
    #[ignore = "reopens the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    fn reopens_attached_blackwidow_v4_pro_with_stable_identity() {
        let query_serial = || {
            let channel = LiveKeyboard::open();
            execute(
                channel.as_ref(),
                &Command::serial(TEST_TRANSACTION_ID).expect("serial command should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("serial query should succeed")
            .expect("serial query should return a response")
        };

        assert_eq!(query_serial(), query_serial());
    }

    #[test]
    #[ignore = "power-cycles and changes lighting on the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    #[allow(
        clippy::too_many_lines,
        reason = "the opt-in live test keeps its complete mutation, power-cycle, validation, and restoration sequence together"
    )]
    fn validates_attached_blackwidow_v4_pro_power_cycle() {
        const TEST_BRIGHTNESS: u8 = 173;

        let channel = LiveKeyboard::open();
        let original_serial = execute(
            channel.as_ref(),
            &Command::serial(TEST_TRANSACTION_ID).expect("serial command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("serial query should succeed")
        .expect("serial query should return a response");
        let original_brightness = execute(
            channel.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness query should succeed")
        .expect("brightness query should return data")
        .get(2)
        .copied()
        .expect("brightness response should contain a value");

        execute(
            channel.as_ref(),
            &Command::set_brightness(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                TEST_BRIGHTNESS,
            )
            .expect("brightness command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("test brightness should succeed");
        execute(
            channel.as_ref(),
            &Command::extended_matrix_effect(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                MatrixEffect::Static(Rgb {
                    red: 255,
                    green: 255,
                    blue: 255,
                }),
            )
            .expect("static effect should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("test effect should succeed");
        drop(channel);

        let reconnected = LiveKeyboard::reconnect_after_physical_disconnect();
        let reconnected_serial = execute(
            reconnected.as_ref(),
            &Command::serial(TEST_TRANSACTION_ID).expect("serial command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("serial query after reconnect should succeed")
        .expect("serial query after reconnect should return a response");
        let reconnected_brightness = execute(
            reconnected.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness query after reconnect should succeed")
        .expect("brightness query after reconnect should return data")
        .get(2)
        .copied()
        .expect("brightness response should contain a value");

        execute(
            reconnected.as_ref(),
            &Command::set_brightness(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                original_brightness,
            )
            .expect("brightness restoration command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness restoration should succeed");
        reconnected.restore_spectrum();

        assert_eq!(
            reconnected_serial, original_serial,
            "protocol identity changed across a physical reconnect"
        );
        assert_eq!(
            reconnected_brightness, TEST_BRIGHTNESS,
            "VARSTORE brightness did not survive a physical reconnect"
        );
    }

    #[test]
    #[ignore = "power-cycles and changes volatile lighting on the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    fn validates_attached_blackwidow_v4_pro_volatile_storage() {
        const TEST_BRIGHTNESS: u8 = 91;

        let channel = LiveKeyboard::open();
        let persistent_brightness = execute(
            channel.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("persistent brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("persistent brightness query should succeed")
        .expect("persistent brightness query should return data")
        .get(2)
        .copied()
        .expect("persistent brightness response should contain a value");

        execute(
            channel.as_ref(),
            &Command::set_brightness(
                TEST_TRANSACTION_ID,
                Storage::Volatile,
                LedTarget::Backlight,
                TEST_BRIGHTNESS,
            )
            .expect("volatile brightness command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("volatile test brightness should succeed");
        execute(
            channel.as_ref(),
            &Command::extended_matrix_effect(
                TEST_TRANSACTION_ID,
                Storage::Volatile,
                LedTarget::Backlight,
                MatrixEffect::Static(Rgb {
                    red: 0,
                    green: 255,
                    blue: 80,
                }),
            )
            .expect("volatile static effect should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("volatile test effect should succeed");
        drop(channel);

        let reconnected = LiveKeyboard::reconnect_after_physical_disconnect();
        let reconnected_brightness = execute(
            reconnected.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("persistent brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("persistent brightness query after reconnect should succeed")
        .expect("persistent brightness query after reconnect should return data")
        .get(2)
        .copied()
        .expect("persistent brightness response should contain a value");

        assert_eq!(
            reconnected_brightness, persistent_brightness,
            "volatile brightness replaced persistent state across a physical reconnect"
        );
        assert_ne!(
            reconnected_brightness, TEST_BRIGHTNESS,
            "NOSTORE brightness unexpectedly survived a physical reconnect"
        );
    }

    #[test]
    #[ignore = "streams frames to the dedicated 1532:028d BlackWidow V4 Pro test keyboard"]
    fn sustains_attached_blackwidow_v4_pro_complete_frames() {
        const FRAME_COUNT: u8 = 120;
        const MINIMUM_RATE_HZ: f64 = 60.0;

        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");

        let started = Instant::now();
        for frame in 0..FRAME_COUNT {
            for row in 0..8 {
                let colour = Rgb {
                    red: frame,
                    green: row * 31,
                    blue: 255_u8.wrapping_sub(frame),
                };
                execute(
                    channel.as_ref(),
                    &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &[colour; 23])
                        .expect("complete custom row should be valid"),
                    TEST_RESPONSE_DELAY,
                )
                .expect("custom row should succeed");
            }
        }
        let rate = f64::from(FRAME_COUNT) / started.elapsed().as_secs_f64();
        channel.restore_spectrum();
        assert!(
            rate >= MINIMUM_RATE_HZ,
            "complete-frame rate {rate:.1} Hz is below {MINIMUM_RATE_HZ:.1} Hz"
        );
    }

    #[test]
    #[ignore = "visually changes the dedicated 1532:028d test keyboard"]
    fn exercises_attached_blackwidow_v4_pro_static_effect() {
        let channel = LiveKeyboard::open();
        let original_brightness = execute(
            channel.as_ref(),
            &Command::get_brightness(TEST_TRANSACTION_ID, Storage::Variable, LedTarget::Backlight)
                .expect("brightness query should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness query should succeed")
        .expect("brightness query should return data");

        execute(
            channel.as_ref(),
            &Command::set_brightness(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                128,
            )
            .expect("brightness command should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("brightness write should succeed");
        for effect in [
            MatrixEffect::Off,
            MatrixEffect::Static(Rgb {
                red: 255,
                green: 0,
                blue: 0,
            }),
            MatrixEffect::Static(Rgb {
                red: 0,
                green: 255,
                blue: 0,
            }),
            MatrixEffect::Static(Rgb {
                red: 0,
                green: 0,
                blue: 255,
            }),
            MatrixEffect::Off,
        ] {
            execute(
                channel.as_ref(),
                &Command::extended_matrix_effect(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    effect,
                )
                .expect("visual sequence effect should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("visual sequence effect should succeed");
            thread::sleep(Duration::from_secs(2));
        }

        if let Some(brightness) = original_brightness.get(2).copied() {
            execute(
                channel.as_ref(),
                &Command::set_brightness(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    brightness,
                )
                .expect("restoration command should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("brightness restoration should succeed");
        }
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually changes the dedicated 1532:028d test keyboard custom frame"]
    fn exercises_attached_blackwidow_v4_pro_custom_frame() {
        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");
        let cyan = [Rgb {
            red: 0,
            green: 255,
            blue: 255,
        }; 23];
        for row in 0..8 {
            execute(
                channel.as_ref(),
                &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &cyan)
                    .expect("full custom row should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("custom row should succeed");
        }
        thread::sleep(Duration::from_secs(5));
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually exercises BlackWidow V4 Pro firmware effects"]
    fn exercises_attached_blackwidow_v4_pro_firmware_effects() {
        const RED: Rgb = Rgb {
            red: 255,
            green: 0,
            blue: 0,
        };
        const BLUE: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 255,
        };

        let channel = LiveKeyboard::open();
        for effect in [
            MatrixEffect::Wave(Direction::Forward),
            MatrixEffect::Wave(Direction::Reverse),
            MatrixEffect::Wheel(Direction::Forward),
            MatrixEffect::Wheel(Direction::Reverse),
            MatrixEffect::Reactive {
                speed: ReactiveSpeed::Medium,
                colour: RED,
            },
            MatrixEffect::Breathing(ColourMode::Dual(RED, BLUE)),
            MatrixEffect::Starlight {
                speed: StarlightSpeed::Medium,
                colours: ColourMode::Dual(RED, BLUE),
            },
        ] {
            execute(
                channel.as_ref(),
                &Command::extended_matrix_effect(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    effect,
                )
                .expect("firmware effect should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("firmware effect should succeed");
            thread::sleep(Duration::from_secs(4));
        }
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually distinguishes BlackWidow V4 Pro breathing colour modes"]
    fn exercises_attached_blackwidow_v4_pro_breathing_colours() {
        const RED: Rgb = Rgb {
            red: 255,
            green: 0,
            blue: 0,
        };
        const BLUE: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 255,
        };

        let channel = LiveKeyboard::open();
        for colours in [
            ColourMode::Single(RED),
            ColourMode::Single(BLUE),
            ColourMode::Dual(RED, BLUE),
        ] {
            execute(
                channel.as_ref(),
                &Command::extended_matrix_effect(
                    TEST_TRANSACTION_ID,
                    Storage::Variable,
                    LedTarget::Backlight,
                    MatrixEffect::Breathing(colours),
                )
                .expect("breathing effect should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("breathing effect should succeed");
            thread::sleep(Duration::from_secs(8));
        }
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually isolates BlackWidow V4 Pro dual-colour breathing"]
    fn exercises_attached_blackwidow_v4_pro_dual_breathing() {
        const RED: Rgb = Rgb {
            red: 255,
            green: 0,
            blue: 0,
        };
        const BLUE: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 255,
        };

        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::extended_matrix_effect(
                TEST_TRANSACTION_ID,
                Storage::Variable,
                LedTarget::Backlight,
                MatrixEffect::Breathing(ColourMode::Dual(RED, BLUE)),
            )
            .expect("dual breathing effect should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("dual breathing effect should succeed");
        thread::sleep(Duration::from_secs(15));
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "maps one BlackWidow V4 Pro matrix row selected by LUMINATE_RAZER_TEST_ROW"]
    fn maps_attached_blackwidow_v4_pro_matrix_row() {
        const BLACK: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 0,
        };
        const RED: Rgb = Rgb {
            red: 255,
            green: 0,
            blue: 0,
        };

        let selected_row = env::var("LUMINATE_RAZER_TEST_ROW")
            .expect("set LUMINATE_RAZER_TEST_ROW to a value from 0 through 7")
            .parse::<u8>()
            .expect("LUMINATE_RAZER_TEST_ROW must be an integer");
        assert!(selected_row < 8, "matrix row must be from 0 through 7");
        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");
        for row in 0..8 {
            let colours = if row == selected_row {
                [RED; 23]
            } else {
                [BLACK; 23]
            };
            execute(
                channel.as_ref(),
                &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &colours)
                    .expect("mapping row should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("mapping row should succeed");
        }
        thread::sleep(Duration::from_secs(10));
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "maps one BlackWidow V4 Pro matrix coordinate selected by environment"]
    fn maps_attached_blackwidow_v4_pro_matrix_coordinate() {
        const BLACK: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 0,
        };
        const GREEN: Rgb = Rgb {
            red: 0,
            green: 255,
            blue: 0,
        };

        let selected_row = env::var("LUMINATE_RAZER_TEST_ROW")
            .expect("set LUMINATE_RAZER_TEST_ROW to a value from 0 through 7")
            .parse::<u8>()
            .expect("LUMINATE_RAZER_TEST_ROW must be an integer");
        let selected_column = env::var("LUMINATE_RAZER_TEST_COLUMN")
            .expect("set LUMINATE_RAZER_TEST_COLUMN to a value from 0 through 22")
            .parse::<u8>()
            .expect("LUMINATE_RAZER_TEST_COLUMN must be an integer");
        let observation_seconds = env::var("LUMINATE_RAZER_TEST_OBSERVATION_SECONDS")
            .map_or(Ok(10), |value| value.parse::<u64>())
            .expect("LUMINATE_RAZER_TEST_OBSERVATION_SECONDS must be an integer");
        assert!(selected_row < 8, "matrix row must be from 0 through 7");
        assert!(
            selected_column < 23,
            "matrix column must be from 0 through 22"
        );

        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");
        for row in 0..8 {
            let mut colours = [BLACK; 23];
            if row == selected_row {
                colours[usize::from(selected_column)] = GREEN;
            }
            execute(
                channel.as_ref(),
                &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &colours)
                    .expect("mapping row should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("mapping row should succeed");
        }
        thread::sleep(Duration::from_secs(observation_seconds));
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually validates BlackWidow V4 Pro underglow boundaries"]
    fn validates_attached_blackwidow_v4_pro_underglow_boundaries() {
        const BLACK: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 0,
        };
        const GREEN: Rgb = Rgb {
            red: 0,
            green: 255,
            blue: 0,
        };
        const RED: Rgb = Rgb {
            red: 255,
            green: 0,
            blue: 0,
        };
        const BLUE: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 255,
        };

        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");
        for row in 0..8 {
            let mut colours = [BLACK; 23];
            if row == 6 {
                colours[0] = GREEN;
                colours[8] = GREEN;
                colours[9] = BLUE;
                colours[17] = BLUE;
                colours[18] = RED;
                colours[22] = RED;
            }
            execute(
                channel.as_ref(),
                &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &colours)
                    .expect("mapping row should be valid"),
                TEST_RESPONSE_DELAY,
            )
            .expect("mapping row should succeed");
        }
        thread::sleep(Duration::from_secs(30));
        channel.restore_spectrum();
    }

    #[test]
    #[ignore = "visually maps the BlackWidow V4 Pro right underglow ordering"]
    fn maps_attached_blackwidow_v4_pro_right_underglow_order() {
        const BLACK: Rgb = Rgb {
            red: 0,
            green: 0,
            blue: 0,
        };
        const GREEN: Rgb = Rgb {
            red: 0,
            green: 255,
            blue: 0,
        };

        let channel = LiveKeyboard::open();
        execute(
            channel.as_ref(),
            &Command::custom_mode(TEST_TRANSACTION_ID).expect("custom mode should be valid"),
            TEST_RESPONSE_DELAY,
        )
        .expect("custom mode should succeed");
        for selected_column in 9..=17 {
            for row in 0..8 {
                let mut colours = [BLACK; 23];
                if row == 6 {
                    colours[selected_column] = GREEN;
                }
                execute(
                    channel.as_ref(),
                    &Command::custom_frame_row(TEST_TRANSACTION_ID, row, 0, &colours)
                        .expect("mapping row should be valid"),
                    TEST_RESPONSE_DELAY,
                )
                .expect("mapping row should succeed");
            }
            thread::sleep(Duration::from_secs(3));
        }
        channel.restore_spectrum();
    }
}

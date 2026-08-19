// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Typed discovery for Keychron's common QMK Raw HID protocol.

use std::error::Error;
use std::fmt;

use crate::transport::{REPORT_LENGTH, Transport};

const GET_PROTOCOL_VERSION: u8 = 0xa0;
const GET_FIRMWARE_VERSION: u8 = 0xa1;
const GET_SUPPORTED_FEATURES: u8 = 0xa2;
const KEYCHRON_RGB: u8 = 0xa8;

const RGB_GET_PROTOCOL_VERSION: u8 = 0x01;
const RGB_SAVE: u8 = 0x02;
const RGB_GET_LED_COUNT: u8 = 0x05;
const PER_KEY_RGB_SET_TYPE: u8 = 0x08;
const PER_KEY_RGB_SET_COLOR: u8 = 0x0a;

const PER_KEY_RGB_SOLID: u8 = 0;
const MAX_COLOURS_PER_REPORT: usize = 9;

const KEYCHRON_PROTOCOL_VERSION: u8 = 0x02;
const QMK_COMMAND_SET: u8 = 0x02;
const FEATURE_KEYCHRON_RGB: u8 = 1 << 7;
const RGB_PROTOCOL_VERSION: u16 = 0x0001;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Discovery {
    pub(crate) firmware_version: String,
    pub(crate) rgb_protocol_version: u16,
    pub(crate) led_count: u8,
}

pub(crate) fn discover(transport: &dyn Transport) -> Result<Discovery, ProtocolError> {
    let protocol = exchange(transport, request(GET_PROTOCOL_VERSION, None))?;
    if protocol[1] != KEYCHRON_PROTOCOL_VERSION || protocol[3] != QMK_COMMAND_SET {
        return Err(ProtocolError::UnsupportedProtocol {
            protocol: protocol[1],
            command_set: protocol[3],
        });
    }

    let features = exchange(transport, request(GET_SUPPORTED_FEATURES, None))?;
    if features[1] & FEATURE_KEYCHRON_RGB == 0 {
        return Err(ProtocolError::RgbUnsupported);
    }

    let firmware = exchange(transport, request(GET_FIRMWARE_VERSION, None))?;
    let firmware_version = parse_firmware_version(&firmware[1..]);

    let rgb_version = exchange(
        transport,
        request(KEYCHRON_RGB, Some(RGB_GET_PROTOCOL_VERSION)),
    )?;
    validate_rgb_response(&rgb_version, RGB_GET_PROTOCOL_VERSION)?;
    let rgb_protocol_version = u16::from_le_bytes([rgb_version[3], rgb_version[4]]);
    if rgb_protocol_version != RGB_PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedRgbProtocol(rgb_protocol_version));
    }

    let led_count = exchange(transport, request(KEYCHRON_RGB, Some(RGB_GET_LED_COUNT)))?;
    validate_rgb_response(&led_count, RGB_GET_LED_COUNT)?;
    if led_count[3] == 0 {
        return Err(ProtocolError::InvalidLedCount);
    }

    Ok(Discovery {
        firmware_version,
        rgb_protocol_version,
        led_count: led_count[3],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hsv {
    pub(crate) hue: u8,
    pub(crate) saturation: u8,
    pub(crate) value: u8,
}

pub(crate) fn set_per_key_colours(
    transport: &dyn Transport,
    led_count: u8,
    start: u8,
    colours: &[Hsv],
) -> Result<(), ProtocolError> {
    let end =
        usize::from(start)
            .checked_add(colours.len())
            .ok_or(ProtocolError::InvalidColourRange {
                start,
                count: colours.len(),
                led_count,
            })?;
    if colours.is_empty() || end > usize::from(led_count) {
        return Err(ProtocolError::InvalidColourRange {
            start,
            count: colours.len(),
            led_count,
        });
    }

    let mut type_request = request(KEYCHRON_RGB, Some(PER_KEY_RGB_SET_TYPE));
    type_request[2] = PER_KEY_RGB_SOLID;
    let type_response = exchange(transport, type_request)?;
    validate_rgb_response(&type_response, PER_KEY_RGB_SET_TYPE)?;

    for (chunk_index, chunk) in colours.chunks(MAX_COLOURS_PER_REPORT).enumerate() {
        let offset = chunk_index
            .checked_mul(MAX_COLOURS_PER_REPORT)
            .and_then(|offset| usize::from(start).checked_add(offset))
            .and_then(|offset| u8::try_from(offset).ok())
            .ok_or(ProtocolError::InvalidColourRange {
                start,
                count: colours.len(),
                led_count,
            })?;
        let count = u8::try_from(chunk.len()).map_err(|_| ProtocolError::InvalidColourRange {
            start,
            count: colours.len(),
            led_count,
        })?;
        let mut colour_request = request(KEYCHRON_RGB, Some(PER_KEY_RGB_SET_COLOR));
        colour_request[2] = offset;
        colour_request[3] = count;
        for (index, colour) in chunk.iter().enumerate() {
            let base = 4 + index * 3;
            if let Some(bytes) = colour_request.get_mut(base..base + 3) {
                bytes.copy_from_slice(&[colour.hue, colour.saturation, colour.value]);
            }
        }
        let colour_response = exchange(transport, colour_request)?;
        validate_rgb_response(&colour_response, PER_KEY_RGB_SET_COLOR)?;
    }

    Ok(())
}

pub(crate) fn save(transport: &dyn Transport) -> Result<(), ProtocolError> {
    let response = exchange(transport, request(KEYCHRON_RGB, Some(RGB_SAVE)))?;
    validate_rgb_response(&response, RGB_SAVE)
}

fn request(command: u8, subcommand: Option<u8>) -> [u8; REPORT_LENGTH] {
    let mut report = [0; REPORT_LENGTH];
    report[0] = command;
    if let Some(subcommand) = subcommand {
        report[1] = subcommand;
    }
    report
}

fn exchange(
    transport: &dyn Transport,
    request: [u8; REPORT_LENGTH],
) -> Result<[u8; REPORT_LENGTH], ProtocolError> {
    let expected_command = request[0];
    let response = transport
        .transact(request)
        .map_err(ProtocolError::Transport)?;
    if response[0] != expected_command {
        return Err(ProtocolError::UnexpectedCommand {
            expected: expected_command,
            actual: response[0],
        });
    }
    Ok(response)
}

fn validate_rgb_response(
    response: &[u8; REPORT_LENGTH],
    subcommand: u8,
) -> Result<(), ProtocolError> {
    if response[1] != subcommand {
        return Err(ProtocolError::UnexpectedSubcommand {
            expected: subcommand,
            actual: response[1],
        });
    }
    if response[2] != 0 {
        return Err(ProtocolError::CommandFailed {
            subcommand,
            status: response[2],
        });
    }
    Ok(())
}

fn parse_firmware_version(bytes: &[u8]) -> String {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    bytes
        .get(..length)
        .map(String::from_utf8_lossy)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    Transport(String),
    UnexpectedCommand {
        expected: u8,
        actual: u8,
    },
    UnexpectedSubcommand {
        expected: u8,
        actual: u8,
    },
    UnsupportedProtocol {
        protocol: u8,
        command_set: u8,
    },
    UnsupportedRgbProtocol(u16),
    RgbUnsupported,
    CommandFailed {
        subcommand: u8,
        status: u8,
    },
    InvalidLedCount,
    InvalidColourRange {
        start: u8,
        count: usize,
        led_count: u8,
    },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) => formatter.write_str(message),
            Self::UnexpectedCommand { expected, actual } => write!(
                formatter,
                "Keychron response command is {actual:#04x}, expected {expected:#04x}"
            ),
            Self::UnexpectedSubcommand { expected, actual } => write!(
                formatter,
                "Keychron RGB response subcommand is {actual:#04x}, expected {expected:#04x}"
            ),
            Self::UnsupportedProtocol {
                protocol,
                command_set,
            } => write!(
                formatter,
                "unsupported Keychron protocol {protocol:#04x} with QMK command set {command_set:#04x}"
            ),
            Self::UnsupportedRgbProtocol(version) => {
                write!(
                    formatter,
                    "unsupported Keychron RGB protocol {version:#06x}"
                )
            }
            Self::RgbUnsupported => {
                formatter.write_str("Keychron firmware does not advertise the common RGB protocol")
            }
            Self::CommandFailed { subcommand, status } => write!(
                formatter,
                "Keychron RGB subcommand {subcommand:#04x} failed with status {status:#04x}"
            ),
            Self::InvalidLedCount => {
                formatter.write_str("Keychron RGB protocol reported zero LEDs")
            }
            Self::InvalidColourRange {
                start,
                count,
                led_count,
            } => write!(
                formatter,
                "Keychron colour range starts at {start} with {count} LEDs, but the device has {led_count}"
            ),
        }
    }
}

impl Error for ProtocolError {}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;

    struct MockTransport {
        responses: RefCell<VecDeque<Result<[u8; REPORT_LENGTH], String>>>,
        requests: RefCell<Vec<[u8; REPORT_LENGTH]>>,
    }

    impl MockTransport {
        fn new(responses: Vec<[u8; REPORT_LENGTH]>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().map(Ok).collect()),
                requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl Transport for MockTransport {
        fn transact(&self, request: [u8; REPORT_LENGTH]) -> Result<[u8; REPORT_LENGTH], String> {
            self.requests.borrow_mut().push(request);
            self.responses
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| "mock received an unexpected request".to_owned())?
        }
    }

    fn response(command: u8) -> [u8; REPORT_LENGTH] {
        let mut report = [0; REPORT_LENGTH];
        report[0] = command;
        report
    }

    fn successful_responses() -> Vec<[u8; REPORT_LENGTH]> {
        let mut protocol = response(GET_PROTOCOL_VERSION);
        protocol[1] = KEYCHRON_PROTOCOL_VERSION;
        protocol[3] = QMK_COMMAND_SET;

        let mut features = response(GET_SUPPORTED_FEATURES);
        features[1] = FEATURE_KEYCHRON_RGB;

        let mut firmware = response(GET_FIRMWARE_VERSION);
        firmware[1..7].copy_from_slice(b"v1.2.3");

        let mut rgb_version = response(KEYCHRON_RGB);
        rgb_version[1] = RGB_GET_PROTOCOL_VERSION;
        rgb_version[3..5].copy_from_slice(&RGB_PROTOCOL_VERSION.to_le_bytes());

        let mut led_count = response(KEYCHRON_RGB);
        led_count[1] = RGB_GET_LED_COUNT;
        led_count[3] = 81;

        vec![protocol, features, firmware, rgb_version, led_count]
    }

    #[test]
    fn discovers_the_mock_common_rgb_device() {
        let transport = MockTransport::new(successful_responses());

        let discovery = discover(&transport).expect("mock discovery should succeed");

        assert_eq!(
            discovery,
            Discovery {
                firmware_version: "v1.2.3".to_owned(),
                rgb_protocol_version: 1,
                led_count: 81,
            }
        );
        let requests = transport.requests.borrow();
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[0][0], GET_PROTOCOL_VERSION);
        assert_eq!(requests[3][..2], [KEYCHRON_RGB, RGB_GET_PROTOCOL_VERSION]);
        assert_eq!(requests[4][..2], [KEYCHRON_RGB, RGB_GET_LED_COUNT]);
    }

    #[test]
    fn rejects_firmware_without_the_common_rgb_feature() {
        let mut responses = successful_responses();
        responses[1][1] = 0;
        let transport = MockTransport::new(responses);

        assert_eq!(discover(&transport), Err(ProtocolError::RgbUnsupported));
        assert_eq!(transport.requests.borrow().len(), 2);
    }

    #[test]
    fn rejects_an_unknown_rgb_protocol_version() {
        let mut responses = successful_responses();
        responses[3][3..5].copy_from_slice(&2_u16.to_le_bytes());
        let transport = MockTransport::new(responses);

        assert_eq!(
            discover(&transport),
            Err(ProtocolError::UnsupportedRgbProtocol(2))
        );
    }

    #[test]
    fn rejects_a_failed_rgb_command() {
        let mut responses = successful_responses();
        responses[4][2] = 1;
        let transport = MockTransport::new(responses);

        assert_eq!(
            discover(&transport),
            Err(ProtocolError::CommandFailed {
                subcommand: RGB_GET_LED_COUNT,
                status: 1,
            })
        );
    }

    #[test]
    fn rejects_a_mismatched_response() {
        let mut responses = successful_responses();
        responses[0][0] = GET_FIRMWARE_VERSION;
        let transport = MockTransport::new(responses);

        assert_eq!(
            discover(&transport),
            Err(ProtocolError::UnexpectedCommand {
                expected: GET_PROTOCOL_VERSION,
                actual: GET_FIRMWARE_VERSION,
            })
        );
    }

    #[test]
    fn per_key_colours_are_chunked_without_saving() {
        let mut type_response = response(KEYCHRON_RGB);
        type_response[1] = PER_KEY_RGB_SET_TYPE;
        let mut first_response = response(KEYCHRON_RGB);
        first_response[1] = PER_KEY_RGB_SET_COLOR;
        let mut second_response = first_response;
        second_response[1] = PER_KEY_RGB_SET_COLOR;
        let transport = MockTransport::new(vec![type_response, first_response, second_response]);
        let colours = (0..10)
            .map(|value| Hsv {
                hue: value,
                saturation: value + 1,
                value: value + 2,
            })
            .collect::<Vec<_>>();

        set_per_key_colours(&transport, 81, 7, &colours)
            .expect("the mock colour update should succeed");

        let requests = transport.requests.borrow();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0][..3], [KEYCHRON_RGB, PER_KEY_RGB_SET_TYPE, 0]);
        assert_eq!(
            requests[1][..4],
            [KEYCHRON_RGB, PER_KEY_RGB_SET_COLOR, 7, 9]
        );
        assert_eq!(requests[1][4..7], [0, 1, 2]);
        assert_eq!(requests[1][28..31], [8, 9, 10]);
        assert_eq!(
            requests[2][..4],
            [KEYCHRON_RGB, PER_KEY_RGB_SET_COLOR, 16, 1]
        );
        assert_eq!(requests[2][4..7], [9, 10, 11]);
        assert!(requests.iter().all(|request| request[1] != RGB_SAVE));
    }

    #[test]
    fn rejects_an_out_of_bounds_colour_range_before_transport() {
        let transport = MockTransport::new(Vec::new());

        assert_eq!(
            set_per_key_colours(
                &transport,
                81,
                80,
                &[
                    Hsv {
                        hue: 0,
                        saturation: 0,
                        value: 0,
                    },
                    Hsv {
                        hue: 1,
                        saturation: 1,
                        value: 1,
                    },
                ],
            ),
            Err(ProtocolError::InvalidColourRange {
                start: 80,
                count: 2,
                led_count: 81,
            })
        );
        assert!(transport.requests.borrow().is_empty());
    }

    #[test]
    fn save_uses_the_explicit_firmware_command() {
        let mut save_response = response(KEYCHRON_RGB);
        save_response[1] = RGB_SAVE;
        let transport = MockTransport::new(vec![save_response]);

        save(&transport).expect("the mock save should succeed");

        let requests = transport.requests.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0][..2], [KEYCHRON_RGB, RGB_SAVE]);
    }
}

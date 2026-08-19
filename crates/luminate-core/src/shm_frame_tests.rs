// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

use super::*;
use crate::rgb::Rgb;

#[test]
fn rgb8_round_trips_through_pack_and_unpack() {
    let colour = Colour::rgb(Rgb {
        r: 10,
        g: 20,
        b: 30,
    });
    let mut bytes = [0_u8; 3];

    ShmPixelFormat::Rgb8
        .pack(&colour, &mut bytes)
        .expect("pack");

    assert_eq!(bytes, [10, 20, 30]);
    assert_eq!(ShmPixelFormat::Rgb8.unpack(&bytes), colour);
}

#[test]
fn rgbw8_round_trips_through_pack_and_unpack() {
    let colour = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
        ColourChannelValue::new(ColourChannel::Blue, 3),
        ColourChannelValue::new(ColourChannel::White, 4),
    ])
    .expect("RGBW is valid");
    let mut bytes = [0_u8; 4];

    ShmPixelFormat::Rgbw8
        .pack(&colour, &mut bytes)
        .expect("pack");

    assert_eq!(bytes, [1, 2, 3, 4]);
    assert_eq!(ShmPixelFormat::Rgbw8.unpack(&bytes), colour);
}

#[test]
fn rgbx8_ignores_and_zeroes_the_padding_byte() {
    let colour = Colour::rgb(Rgb { r: 5, g: 6, b: 7 });
    let mut bytes = [0xff_u8; 4];

    ShmPixelFormat::Rgbx8
        .pack(&colour, &mut bytes)
        .expect("pack");

    assert_eq!(bytes, [5, 6, 7, 0]);
}

#[test]
fn mono8_round_trips_through_pack_and_unpack() {
    let colour = Colour::monochrome(200);
    let mut bytes = [0_u8; 1];

    ShmPixelFormat::Mono8
        .pack(&colour, &mut bytes)
        .expect("pack");

    assert_eq!(bytes, [200]);
    assert_eq!(ShmPixelFormat::Mono8.unpack(&bytes), colour);
}

#[test]
fn pack_rejects_wrong_buffer_length() {
    let colour = Colour::monochrome(1);
    let mut bytes = [0_u8; 2];

    let error = ShmPixelFormat::Mono8.pack(&colour, &mut bytes).unwrap_err();

    assert_eq!(
        error,
        ShmPixelFormatError::BufferLength {
            expected: 1,
            found: 2
        }
    );
}

#[test]
fn pack_rejects_encoding_mismatch() {
    let colour = Colour::monochrome(1);
    let mut bytes = [0_u8; 3];

    let error = ShmPixelFormat::Rgb8.pack(&colour, &mut bytes).unwrap_err();

    assert_eq!(
        error,
        ShmPixelFormatError::EncodingMismatch {
            expected: ColourEncoding::Additive,
            found: ColourEncoding::Monochrome,
        }
    );
}

#[test]
fn pack_rejects_missing_channel() {
    let colour = Colour::additive(vec![
        ColourChannelValue::new(ColourChannel::Red, 1),
        ColourChannelValue::new(ColourChannel::Green, 2),
    ])
    .expect("RG is valid");
    let mut bytes = [0_u8; 3];

    let error = ShmPixelFormat::Rgb8.pack(&colour, &mut bytes).unwrap_err();

    assert_eq!(
        error,
        ShmPixelFormatError::MissingChannel(ColourChannel::Blue)
    );
}

#[test]
fn unpack_treats_a_short_buffer_as_zero_padded_rather_than_panicking() {
    let colour = ShmPixelFormat::Rgb8.unpack(&[9]);

    assert_eq!(
        colour,
        Colour::additive(vec![
            ColourChannelValue::new(ColourChannel::Red, 9),
            ColourChannelValue::new(ColourChannel::Green, 0),
            ColourChannelValue::new(ColourChannel::Blue, 0),
        ])
        .expect("RGB is valid")
    );
}

#[test]
fn abi_discriminants_round_trip() {
    for format in [
        ShmPixelFormat::Rgb8,
        ShmPixelFormat::Rgbw8,
        ShmPixelFormat::Mono8,
        ShmPixelFormat::Rgbx8,
    ] {
        assert_eq!(ShmPixelFormat::from_abi(format.to_abi()), Some(format));
    }
    assert_eq!(ShmPixelFormat::from_abi(999), None);
}

#[test]
fn commit_flag_round_trips() {
    let header = ShmFrameHeader {
        sequence: 0,
        generation: 0,
        pixel_count: 0,
        pixel_format: ShmPixelFormat::Rgb8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    };

    assert!(!header.commit());
    assert!(header.with_commit(true).commit());
    assert!(!header.with_commit(true).with_commit(false).commit());
}

#[test]
fn header_round_trips_through_bytes() {
    let header = ShmFrameHeader {
        sequence: 0x0102_0304_0506_0708,
        generation: 9,
        pixel_count: 16,
        pixel_format: ShmPixelFormat::Rgbw8.to_abi(),
        header_version: SHM_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
    }
    .with_commit(true);

    let bytes = header.to_bytes();

    assert_eq!(bytes.len(), 24);
    let decoded = ShmFrameHeader::from_bytes(&bytes).expect("decode header");
    assert_eq!(decoded.sequence, header.sequence);
    assert_eq!(decoded.generation, header.generation);
    assert_eq!(decoded.pixel_count, header.pixel_count);
    assert_eq!(decoded.pixel_format, header.pixel_format);
    assert_eq!(decoded.header_version, header.header_version);
    assert_eq!(decoded.flags, header.flags);
    assert_eq!(decoded.reserved, header.reserved);
    assert!(decoded.commit());
}

#[test]
fn header_bytes_are_little_endian() {
    let header = ShmFrameHeader {
        sequence: 1,
        generation: 0,
        pixel_count: 0,
        pixel_format: 0,
        header_version: 0,
        flags: 0,
        reserved: 0,
    };

    let bytes = header.to_bytes();

    assert_eq!(&bytes[..8], [1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn header_from_bytes_rejects_a_short_buffer() {
    assert!(ShmFrameHeader::from_bytes(&[0_u8; 23]).is_none());
}

fn client_header(pixel_count: u32, pixel_format: ShmPixelFormat) -> ShmClientFrameHeader {
    ShmClientFrameHeader {
        sequence: 1,
        generation: 1,
        pixel_count,
        pixel_format: pixel_format.to_abi(),
        header_version: SHM_CLIENT_FRAME_HEADER_VERSION,
        flags: 0,
        reserved: 0,
        stream_nonce: 0xdead_beef,
    }
}

#[test]
fn client_header_round_trips_through_bytes() {
    let header = client_header(4, ShmPixelFormat::Rgbw8).with_commit(true);

    let bytes = header.to_bytes();

    assert_eq!(bytes.len(), SHM_CLIENT_FRAME_HEADER_LEN);
    let decoded = ShmClientFrameHeader::from_bytes(&bytes).expect("decode header");
    assert_eq!(decoded.sequence, header.sequence);
    assert_eq!(decoded.generation, header.generation);
    assert_eq!(decoded.pixel_count, header.pixel_count);
    assert_eq!(decoded.pixel_format, header.pixel_format);
    assert_eq!(decoded.header_version, header.header_version);
    assert_eq!(decoded.flags, header.flags);
    assert_eq!(decoded.reserved, header.reserved);
    assert_eq!(decoded.stream_nonce, header.stream_nonce);
    assert!(decoded.commit());
}

#[test]
fn client_header_from_bytes_rejects_a_short_buffer() {
    assert!(ShmClientFrameHeader::from_bytes(&[0_u8; 31]).is_none());
}

#[test]
fn client_header_parse_accepts_a_well_formed_sample() {
    let header = client_header(2, ShmPixelFormat::Rgb8);
    let sample = header.to_bytes();
    let payload = [0_u8; 6];

    let (decoded, format) =
        ShmClientFrameHeader::parse(&sample, &payload).expect("parse should accept");

    assert_eq!(decoded.pixel_count, 2);
    assert_eq!(format, ShmPixelFormat::Rgb8);
}

#[test]
fn client_header_parse_rejects_truncated_sample() {
    let error = ShmClientFrameHeader::parse(&[0_u8; 10], &[]).unwrap_err();
    assert_eq!(error, ShmClientFrameError::TruncatedHeader);
}

#[test]
fn client_header_parse_rejects_bad_header_version() {
    let mut header = client_header(1, ShmPixelFormat::Mono8);
    header.header_version = SHM_CLIENT_FRAME_HEADER_VERSION + 1;
    let sample = header.to_bytes();

    let error = ShmClientFrameHeader::parse(&sample, &[0_u8; 1]).unwrap_err();

    assert_eq!(
        error,
        ShmClientFrameError::UnsupportedHeaderVersion {
            found: SHM_CLIENT_FRAME_HEADER_VERSION + 1
        }
    );
}

#[test]
fn client_header_parse_rejects_unknown_pixel_format() {
    let mut header = client_header(1, ShmPixelFormat::Mono8);
    header.pixel_format = 999;
    let sample = header.to_bytes();

    let error = ShmClientFrameHeader::parse(&sample, &[0_u8; 1]).unwrap_err();

    assert_eq!(
        error,
        ShmClientFrameError::UnknownPixelFormat { found: 999 }
    );
}

#[test]
fn client_header_parse_rejects_unrecognized_flags() {
    let mut header = client_header(1, ShmPixelFormat::Mono8);
    header.flags = 0b1000_0000;
    let sample = header.to_bytes();

    let error = ShmClientFrameHeader::parse(&sample, &[0_u8; 1]).unwrap_err();

    assert_eq!(
        error,
        ShmClientFrameError::UnrecognizedFlags { found: 0b1000_0000 }
    );
}

#[test]
fn client_header_parse_accepts_the_largest_possible_pixel_count_without_panicking() {
    // The multiplication cannot overflow `usize` on a 64-bit target. This
    // exercises the largest valid `pixel_count` and verifies that its absent
    // payload produces a length error without panicking.
    let header = client_header(u32::MAX, ShmPixelFormat::Rgbw8);
    let sample = header.to_bytes();

    let error = ShmClientFrameHeader::parse(&sample, &[]).unwrap_err();

    assert_eq!(
        error,
        ShmClientFrameError::PayloadLengthMismatch {
            expected: usize::try_from(u32::MAX).expect("u32::MAX fits in usize") * 4,
            found: 0,
        }
    );
}

#[test]
fn client_header_parse_rejects_payload_length_mismatch() {
    let header = client_header(2, ShmPixelFormat::Rgb8);
    let sample = header.to_bytes();

    let error = ShmClientFrameHeader::parse(&sample, &[0_u8; 5]).unwrap_err();

    assert_eq!(
        error,
        ShmClientFrameError::PayloadLengthMismatch {
            expected: 6,
            found: 5,
        }
    );
}

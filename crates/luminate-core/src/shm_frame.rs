// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Compact, fixed-layout pixel data for the zero-copy shared-memory frame
//! fast path.
//!
//! [`crate::frame::FrameEnvelope`]'s `Colour`-based payload is
//! self-describing and variable-length, which suits the ordinary
//! request/response frame-upload path but is unsuitable for a shared-memory
//! ring buffer, which needs a fixed byte stride per pixel known ahead of
//! time. The types here give the fast path its own compact wire shape while
//! staying deliberately independent of any particular transport: nothing in
//! this module knows about shared memory, sockets, or pipes.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::capability::{ColourChannel, ColourEncoding};
use crate::colour::{Colour, ColourChannelValue};
use crate::rgb::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
/// A fixed-stride pixel encoding usable in a shared-memory frame buffer.
///
/// Deliberately small: it covers what bundled plugins' colour capabilities
/// actually use today. Add a variant, and bump `PLUGIN_ABI_VERSION`, when a
/// consumer needs a format not represented here.
pub enum ShmPixelFormat {
    /// Red, green, blue, additive, 8 bits each. 3 bytes per pixel.
    Rgb8 = 0,

    /// Red, green, blue, white, additive, 8 bits each. 4 bytes per pixel.
    Rgbw8 = 1,

    /// A single monochrome intensity channel, 8 bits. 1 byte per pixel.
    Mono8 = 2,

    /// Red, green, blue, 8 bits each, plus one unused padding byte. 4 bytes
    /// per pixel. Unlike [`Self::Rgbw8`], the fourth byte carries no
    /// meaningful channel data; it exists purely so producers that prefer
    /// 32-bit-aligned pixel strides don't pay [`Self::Rgb8`]'s 3-byte
    /// misalignment.
    Rgbx8 = 3,
}

impl ShmPixelFormat {
    /// Number of bytes one pixel occupies in this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb8 => 3,
            Self::Rgbw8 | Self::Rgbx8 => 4,
            Self::Mono8 => 1,
        }
    }

    /// The colour encoding this format's channels are expressed in.
    #[must_use]
    const fn encoding(self) -> ColourEncoding {
        match self {
            Self::Rgb8 | Self::Rgbw8 | Self::Rgbx8 => ColourEncoding::Additive,
            Self::Mono8 => ColourEncoding::Monochrome,
        }
    }

    /// The stable discriminant carried across the plugin ABI and the
    /// daemon ↔ plugin-host control protocol.
    #[must_use]
    pub const fn to_abi(self) -> u32 {
        self as u32
    }

    /// Recovers a format from an ABI discriminant, rejecting values that
    /// don't name a known format (for example, one introduced by a newer
    /// peer).
    #[must_use]
    pub const fn from_abi(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Rgb8),
            1 => Some(Self::Rgbw8),
            2 => Some(Self::Mono8),
            3 => Some(Self::Rgbx8),
            _ => None,
        }
    }

    /// Packs one [`Colour`] into `out`, which must be exactly
    /// [`Self::bytes_per_pixel`] bytes long.
    ///
    /// # Errors
    ///
    /// Returns an error if `out` has the wrong length, `colour`'s encoding
    /// doesn't match this format, or `colour` is missing a channel this
    /// format requires.
    pub fn pack(self, colour: &Colour, out: &mut [u8]) -> Result<(), ShmPixelFormatError> {
        let expected_len = self.bytes_per_pixel();
        if out.len() != expected_len {
            return Err(ShmPixelFormatError::BufferLength {
                expected: expected_len,
                found: out.len(),
            });
        }

        let expected_encoding = self.encoding();
        if colour.encoding() != expected_encoding {
            return Err(ShmPixelFormatError::EncodingMismatch {
                expected: expected_encoding,
                found: colour.encoding(),
            });
        }

        let channel_byte = |channel: ColourChannel| -> Result<u8, ShmPixelFormatError> {
            let channels = match colour {
                Colour::Additive(channels) => channels.as_slice(),
                Colour::Monochrome { intensity } if channel == ColourChannel::Intensity => {
                    return Ok(u8::try_from(*intensity).unwrap_or(u8::MAX));
                }
                Colour::Hsv { .. }
                | Colour::Hsl { .. }
                | Colour::Cct { .. }
                | Colour::Monochrome { .. } => {
                    return Err(ShmPixelFormatError::MissingChannel(channel));
                }
            };
            channels
                .iter()
                .find(|value| value.channel == channel)
                .map(|value| u8::try_from(value.value).unwrap_or(u8::MAX))
                .ok_or(ShmPixelFormatError::MissingChannel(channel))
        };

        // Assemble every format in a four-byte scratch buffer, then copy only
        // the format's validated stride. This avoids unchecked indexing.
        let bytes: [u8; 4] = match self {
            // Rgb8 and Rgbx8 pack identically; only their byte stride differs.
            Self::Rgb8 | Self::Rgbx8 => [
                channel_byte(ColourChannel::Red)?,
                channel_byte(ColourChannel::Green)?,
                channel_byte(ColourChannel::Blue)?,
                0,
            ],
            Self::Rgbw8 => [
                channel_byte(ColourChannel::Red)?,
                channel_byte(ColourChannel::Green)?,
                channel_byte(ColourChannel::Blue)?,
                channel_byte(ColourChannel::White)?,
            ],
            Self::Mono8 => [channel_byte(ColourChannel::Intensity)?, 0, 0, 0],
        };

        for (slot, value) in out.iter_mut().zip(bytes.iter()) {
            *slot = *value;
        }

        Ok(())
    }

    /// Reconstructs a [`Colour`] from packed pixel bytes.
    ///
    /// A convenience for callers that want a [`Colour`] rather than
    /// hand-rolled byte math (tests, diagnostics). Missing bytes are
    /// treated as zero rather than causing a panic, since `bytes` may
    /// originate from a peer this process doesn't fully trust; callers that
    /// need to detect a malformed buffer should check its length against
    /// [`Self::bytes_per_pixel`] themselves before calling.
    #[must_use]
    pub fn unpack(self, bytes: &[u8]) -> Colour {
        let byte = |index: usize| bytes.get(index).copied().unwrap_or(0);

        let channels = match self {
            // Rgb8 and Rgbx8 unpack identically; Rgbx8's fourth byte is
            // padding and carries no channel.
            Self::Rgb8 | Self::Rgbx8 => vec![
                ColourChannelValue::new(ColourChannel::Red, byte(0).into()),
                ColourChannelValue::new(ColourChannel::Green, byte(1).into()),
                ColourChannelValue::new(ColourChannel::Blue, byte(2).into()),
            ],
            Self::Rgbw8 => vec![
                ColourChannelValue::new(ColourChannel::Red, byte(0).into()),
                ColourChannelValue::new(ColourChannel::Green, byte(1).into()),
                ColourChannelValue::new(ColourChannel::Blue, byte(2).into()),
                ColourChannelValue::new(ColourChannel::White, byte(3).into()),
            ],
            Self::Mono8 => vec![ColourChannelValue::new(
                ColourChannel::Intensity,
                byte(0).into(),
            )],
        };

        match self {
            Self::Mono8 => Colour::monochrome(u32::from(byte(0))),
            Self::Rgb8 | Self::Rgbw8 | Self::Rgbx8 => {
                // Every packed additive format above supplies at least one
                // unique emitter, so construction cannot fail.
                Colour::additive(channels).unwrap_or_else(|_| Colour::rgb(Rgb::new(0, 0, 0)))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// A [`ShmPixelFormat::pack`] failure.
pub enum ShmPixelFormatError {
    /// The colour's encoding doesn't match what this format requires.
    #[error("colour encoding {found:?} does not match the format's required {expected:?}")]
    EncodingMismatch {
        /// Encoding this format requires.
        expected: ColourEncoding,
        /// Encoding the colour actually carried.
        found: ColourEncoding,
    },

    /// The colour is missing a channel this format requires.
    #[error("colour is missing required channel {0:?}")]
    MissingChannel(ColourChannel),

    /// The destination buffer isn't exactly `bytes_per_pixel()` long.
    #[error("expected a {expected}-byte pixel buffer, found {found} bytes")]
    BufferLength {
        /// Length `bytes_per_pixel()` requires.
        expected: usize,
        /// Length actually supplied.
        found: usize,
    },
}

/// Version of [`ShmFrameHeader`]'s own byte layout, independent of
/// `PLUGIN_ABI_VERSION`: this covers only the header's binary shape, not
/// the wider plugin ABI or control protocol built on top of it.
pub const SHM_FRAME_HEADER_VERSION: u16 = 1;

const COMMIT_FLAG: u8 = 0b0000_0001;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// Fixed 24-byte header written at the front of every published sample in
/// the shared-memory frame fast path, immediately followed by
/// `pixel_count * pixel_format.bytes_per_pixel()` tightly packed pixel
/// bytes.
///
/// Mirrors [`crate::frame::FrameEnvelope`]'s `generation`, `sequence`, and
/// `commit` fields exactly, so the fast path shares the ordinary
/// request/response path's staleness and ordering vocabulary rather than
/// inventing a second one.
pub struct ShmFrameHeader {
    /// Monotonically increasing within one `generation`; detects
    /// out-of-order or duplicate samples. See
    /// [`crate::frame::FrameEnvelope::sequence`].
    pub sequence: u64,

    /// Identifies the stream this sample belongs to; bumped each time a
    /// stream (re)starts. See
    /// [`crate::frame::FrameEnvelope::generation`].
    pub generation: u32,

    /// Number of pixels in the buffer following this header.
    pub pixel_count: u32,

    /// The pixel format the following buffer is encoded in, as a
    /// [`ShmPixelFormat`] discriminant (see
    /// [`ShmPixelFormat::to_abi`]/[`ShmPixelFormat::from_abi`]).
    pub pixel_format: u32,

    /// [`SHM_FRAME_HEADER_VERSION`] this sample was written under.
    pub header_version: u16,

    /// Bit 0 mirrors [`crate::frame::FrameEnvelope::commit`]; remaining
    /// bits are reserved and must be zero.
    pub flags: u8,

    /// Padding to keep the header's size a multiple of its alignment.
    /// Always zero; not currently interpreted.
    pub reserved: u8,
}

const _: () = {
    assert!(
        size_of::<ShmFrameHeader>() == 24,
        "ShmFrameHeader must keep its documented 24-byte wire layout"
    );
    assert!(
        align_of::<ShmFrameHeader>() == 8,
        "ShmFrameHeader must keep its documented 8-byte alignment"
    );
};

impl ShmFrameHeader {
    /// Reads the `commit` flag out of `flags`.
    #[must_use]
    pub const fn commit(self) -> bool {
        self.flags & COMMIT_FLAG != 0
    }

    /// Returns a copy of this header with the `commit` flag set to
    /// `commit`.
    #[must_use]
    pub const fn with_commit(mut self, commit: bool) -> Self {
        if commit {
            self.flags |= COMMIT_FLAG;
        } else {
            self.flags &= !COMMIT_FLAG;
        }
        self
    }

    /// Encodes this header as its 24-byte little-endian wire form.
    ///
    /// Deliberately explicit field-by-field encoding rather than a raw
    /// `repr(C)` memory reinterpretation: a shared-memory payload buffer
    /// (for example, an iceoryx2 `[u8]` service payload) is only guaranteed
    /// byte alignment, not this struct's natural 8-byte alignment, so
    /// reading it back through a pointer cast would risk unaligned-access
    /// undefined behaviour. Byte-by-byte (de)serialization sidesteps that
    /// entirely and is also endian-portable.
    #[must_use]
    pub fn to_bytes(self) -> [u8; 24] {
        let mut bytes = [0_u8; 24];
        let (sequence, rest) = bytes.split_at_mut(8);
        sequence.copy_from_slice(&self.sequence.to_le_bytes());
        let (generation, rest) = rest.split_at_mut(4);
        generation.copy_from_slice(&self.generation.to_le_bytes());
        let (pixel_count, rest) = rest.split_at_mut(4);
        pixel_count.copy_from_slice(&self.pixel_count.to_le_bytes());
        let (pixel_format, rest) = rest.split_at_mut(4);
        pixel_format.copy_from_slice(&self.pixel_format.to_le_bytes());
        let (header_version, rest) = rest.split_at_mut(2);
        header_version.copy_from_slice(&self.header_version.to_le_bytes());
        let (flags, reserved) = rest.split_at_mut(1);
        flags.copy_from_slice(&[self.flags]);
        reserved.copy_from_slice(&[self.reserved]);
        bytes
    }

    /// Decodes a header from its 24-byte little-endian wire form. Returns
    /// `None` if `bytes` is shorter than 24 bytes; every byte pattern in a
    /// buffer of at least that length decodes to some header (individual
    /// field values, such as an unrecognized `pixel_format`, are validated
    /// by their own dedicated decoders, not here).
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..24)?;
        let (sequence, rest) = bytes.split_at(8);
        let (generation, rest) = rest.split_at(4);
        let (pixel_count, rest) = rest.split_at(4);
        let (pixel_format, rest) = rest.split_at(4);
        let (header_version, rest) = rest.split_at(2);
        let (flags, reserved) = rest.split_at(1);
        Some(Self {
            sequence: u64::from_le_bytes(sequence.try_into().ok()?),
            generation: u32::from_le_bytes(generation.try_into().ok()?),
            pixel_count: u32::from_le_bytes(pixel_count.try_into().ok()?),
            pixel_format: u32::from_le_bytes(pixel_format.try_into().ok()?),
            header_version: u16::from_le_bytes(header_version.try_into().ok()?),
            flags: *flags.first()?,
            reserved: *reserved.first()?,
        })
    }
}

/// Version of [`ShmClientFrameHeader`]'s own byte layout. Independent of
/// [`SHM_FRAME_HEADER_VERSION`]: the two headers cover different legs of the
/// SHM fast path and are free to version separately.
pub const SHM_CLIENT_FRAME_HEADER_VERSION: u16 = 1;

/// Byte length of [`ShmClientFrameHeader::to_bytes`]/`from_bytes`.
pub const SHM_CLIENT_FRAME_HEADER_LEN: usize = 32;

const CLIENT_COMMIT_FLAG: u8 = 0b0000_0001;
const CLIENT_KNOWN_FLAGS: u8 = CLIENT_COMMIT_FLAG;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
/// Fixed 32-byte header written at the front of every sample a client
/// publishes on the client → daemon leg of the SHM fast path, immediately
/// followed by `pixel_count * pixel_format.bytes_per_pixel()` tightly packed
/// pixel bytes.
///
/// A distinct type from [`ShmFrameHeader`] rather than a shape change to it:
/// `ShmFrameHeader` is already load-bearing on the existing daemon ↔ plugin-host
/// leg, and this leg needs one additional field ([`Self::stream_nonce`]) that
/// leg has no use for. Field-for-field, everything but the trailing nonce
/// mirrors `ShmFrameHeader` exactly, for the same reasons documented there.
pub struct ShmClientFrameHeader {
    /// See [`ShmFrameHeader::sequence`].
    pub sequence: u64,

    /// See [`ShmFrameHeader::generation`].
    pub generation: u32,

    /// See [`ShmFrameHeader::pixel_count`].
    pub pixel_count: u32,

    /// See [`ShmFrameHeader::pixel_format`].
    pub pixel_format: u32,

    /// [`SHM_CLIENT_FRAME_HEADER_VERSION`] this sample was written under.
    pub header_version: u16,

    /// Bit 0 mirrors [`crate::frame::FrameEnvelope::commit`]; remaining bits
    /// are reserved and must be zero.
    pub flags: u8,

    /// Padding to keep the header's size a multiple of its alignment.
    /// Always zero; not currently interpreted.
    pub reserved: u8,

    /// Identifies one `BeginShmFrameStream` negotiation. Catches a stale
    /// same-uid segment: a crashed and respawned client process reusing the
    /// same deterministic, target-derived service name before the daemon has
    /// finished tearing down the previous stream's resources. The daemon
    /// rejects any sample whose nonce doesn't match the one it handed out at
    /// negotiation time; this type only carries the value; comparing it
    /// against the expected nonce is the caller's responsibility, since that
    /// expectation lives in per-stream daemon state this module doesn't have.
    pub stream_nonce: u64,
}

const _: () = {
    assert!(
        size_of::<ShmClientFrameHeader>() == SHM_CLIENT_FRAME_HEADER_LEN,
        "ShmClientFrameHeader must keep its documented 32-byte wire layout"
    );
    assert!(
        align_of::<ShmClientFrameHeader>() == 8,
        "ShmClientFrameHeader must keep its documented 8-byte alignment"
    );
};

impl ShmClientFrameHeader {
    /// Reads the `commit` flag out of `flags`.
    #[must_use]
    pub const fn commit(self) -> bool {
        self.flags & CLIENT_COMMIT_FLAG != 0
    }

    /// Returns a copy of this header with the `commit` flag set to `commit`.
    #[must_use]
    pub const fn with_commit(mut self, commit: bool) -> Self {
        if commit {
            self.flags |= CLIENT_COMMIT_FLAG;
        } else {
            self.flags &= !CLIENT_COMMIT_FLAG;
        }
        self
    }

    /// Encodes this header as its 32-byte little-endian wire form. See
    /// [`ShmFrameHeader::to_bytes`] for why this is explicit field-by-field
    /// encoding rather than a `repr(C)` pointer-cast reinterpretation.
    #[must_use]
    pub fn to_bytes(self) -> [u8; SHM_CLIENT_FRAME_HEADER_LEN] {
        let mut bytes = [0_u8; SHM_CLIENT_FRAME_HEADER_LEN];
        let (sequence, rest) = bytes.split_at_mut(8);
        sequence.copy_from_slice(&self.sequence.to_le_bytes());
        let (generation, rest) = rest.split_at_mut(4);
        generation.copy_from_slice(&self.generation.to_le_bytes());
        let (pixel_count, rest) = rest.split_at_mut(4);
        pixel_count.copy_from_slice(&self.pixel_count.to_le_bytes());
        let (pixel_format, rest) = rest.split_at_mut(4);
        pixel_format.copy_from_slice(&self.pixel_format.to_le_bytes());
        let (header_version, rest) = rest.split_at_mut(2);
        header_version.copy_from_slice(&self.header_version.to_le_bytes());
        let (flags, rest) = rest.split_at_mut(1);
        flags.copy_from_slice(&[self.flags]);
        let (reserved, stream_nonce) = rest.split_at_mut(1);
        reserved.copy_from_slice(&[self.reserved]);
        stream_nonce.copy_from_slice(&self.stream_nonce.to_le_bytes());
        bytes
    }

    /// Decodes a header from its 32-byte little-endian wire form. Returns
    /// `None` if `bytes` is shorter than 32 bytes; this is a length check
    /// only. See [`Self::parse`] for semantic validation of a client-
    /// controlled buffer.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let bytes = bytes.get(..SHM_CLIENT_FRAME_HEADER_LEN)?;
        let (sequence, rest) = bytes.split_at(8);
        let (generation, rest) = rest.split_at(4);
        let (pixel_count, rest) = rest.split_at(4);
        let (pixel_format, rest) = rest.split_at(4);
        let (header_version, rest) = rest.split_at(2);
        let (flags, rest) = rest.split_at(1);
        let (reserved, stream_nonce) = rest.split_at(1);
        Some(Self {
            sequence: u64::from_le_bytes(sequence.try_into().ok()?),
            generation: u32::from_le_bytes(generation.try_into().ok()?),
            pixel_count: u32::from_le_bytes(pixel_count.try_into().ok()?),
            pixel_format: u32::from_le_bytes(pixel_format.try_into().ok()?),
            header_version: u16::from_le_bytes(header_version.try_into().ok()?),
            flags: *flags.first()?,
            reserved: *reserved.first()?,
            stream_nonce: u64::from_le_bytes(stream_nonce.try_into().ok()?),
        })
    }

    /// Parses and fully validates one client-published sample: decodes the
    /// header, then checks everything a hostile or merely buggy same-uid
    /// client could get wrong about it: `header_version`, `pixel_format`,
    /// unrecognized `flags` bits, and (with overflow-checked arithmetic)
    /// that `payload`'s length is exactly `pixel_count * bytes_per_pixel`.
    ///
    /// Deliberately does not check `generation`/`stream_nonce` against an
    /// expected value: those bounds are per-stream daemon state this module
    /// has no access to, so that comparison is the caller's job. Never
    /// panics on any input.
    ///
    /// # Errors
    ///
    /// Returns [`ShmClientFrameError`] describing exactly what was wrong;
    /// see its variants.
    pub fn parse(
        sample: &[u8],
        payload: &[u8],
    ) -> Result<(Self, ShmPixelFormat), ShmClientFrameError> {
        let header = Self::from_bytes(sample).ok_or(ShmClientFrameError::TruncatedHeader)?;

        if header.header_version != SHM_CLIENT_FRAME_HEADER_VERSION {
            return Err(ShmClientFrameError::UnsupportedHeaderVersion {
                found: header.header_version,
            });
        }

        if header.flags & !CLIENT_KNOWN_FLAGS != 0 {
            return Err(ShmClientFrameError::UnrecognizedFlags {
                found: header.flags,
            });
        }

        let format = ShmPixelFormat::from_abi(header.pixel_format).ok_or(
            ShmClientFrameError::UnknownPixelFormat {
                found: header.pixel_format,
            },
        )?;

        let bytes_per_pixel = format.bytes_per_pixel();
        let expected_len = usize::try_from(header.pixel_count)
            .ok()
            .and_then(|count| count.checked_mul(bytes_per_pixel))
            .ok_or(ShmClientFrameError::PixelCountOverflow {
                pixel_count: header.pixel_count,
                bytes_per_pixel,
            })?;

        if payload.len() != expected_len {
            return Err(ShmClientFrameError::PayloadLengthMismatch {
                expected: expected_len,
                found: payload.len(),
            });
        }

        Ok((header, format))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
/// A [`ShmClientFrameHeader::parse`] failure. Every variant is a rejection
/// of client-controlled input, not an internal error; none of them indicate
/// a bug in the reader.
pub enum ShmClientFrameError {
    /// The sample is shorter than [`SHM_CLIENT_FRAME_HEADER_LEN`].
    #[error("sample is shorter than the {SHM_CLIENT_FRAME_HEADER_LEN}-byte client frame header")]
    TruncatedHeader,

    /// `header_version` doesn't match [`SHM_CLIENT_FRAME_HEADER_VERSION`].
    #[error(
        "unsupported client frame header version {found} (expected {SHM_CLIENT_FRAME_HEADER_VERSION})"
    )]
    UnsupportedHeaderVersion {
        /// The unsupported version the sample declared.
        found: u16,
    },

    /// `pixel_format` doesn't name a known [`ShmPixelFormat`] discriminant.
    #[error("unrecognized pixel format discriminant {found}")]
    UnknownPixelFormat {
        /// The unrecognized discriminant.
        found: u32,
    },

    /// `flags` has a bit set outside the set this header version defines.
    #[error("unrecognized flag bits set in {found:#04b}")]
    UnrecognizedFlags {
        /// The full `flags` byte, including its unrecognized bits.
        found: u8,
    },

    /// `pixel_count * bytes_per_pixel` overflows `usize`.
    #[error("pixel_count {pixel_count} * bytes_per_pixel {bytes_per_pixel} overflows usize")]
    PixelCountOverflow {
        /// The declared pixel count.
        pixel_count: u32,
        /// The pixel format's byte stride.
        bytes_per_pixel: usize,
    },

    /// The payload following the header isn't exactly `pixel_count *
    /// bytes_per_pixel` bytes long.
    #[error("expected a {expected}-byte payload, found {found} bytes")]
    PayloadLengthMismatch {
        /// Length `pixel_count * bytes_per_pixel` requires.
        expected: usize,
        /// Length the payload actually was.
        found: usize,
    },
}

#[cfg(test)]
#[path = "shm_frame_tests.rs"]
mod tests;

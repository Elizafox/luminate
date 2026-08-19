// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Small, self-contained ABI enums and byte encodings shared across the
//! plugin descriptor and its callbacks.

/// Informational bus classification for a plugin's hardware. Not currently
/// used by the daemon for filtering; read into `LoadedPluginMetadata` for
/// logging and future discovery tooling.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginBus {
    Unknown = 0,
    Usb = 1,
    Hid = 2,
    I2c = 3,
    Platform = 4,
    Network = 5,
}

impl PluginBus {
    /// This bus's stable ABI code, as it appears in `PluginDescriptor::buses`.
    #[must_use]
    #[inline]
    pub const fn to_abi(self) -> u32 {
        match self {
            Self::Unknown => 0,
            Self::Usb => 1,
            Self::Hid => 2,
            Self::I2c => 3,
            Self::Platform => 4,
            Self::Network => 5,
        }
    }

    /// Parses an untrusted raw ABI code without materializing an invalid enum.
    #[must_use]
    #[inline]
    pub const fn from_abi(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Unknown),
            1 => Some(Self::Usb),
            2 => Some(Self::Hid),
            3 => Some(Self::I2c),
            4 => Some(Self::Platform),
            5 => Some(Self::Network),
            _ => None,
        }
    }
}

/// The kind of match a `PluginProbeHint` represents. Not currently consumed
/// by the loader beyond being read and logged; intended for future
/// discovery tooling that narrows which plugins to try probing.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeHintKind {
    None = 0,
    UsbVidPid = 1,
    HidVidPid = 2,
    DmiMatch = 3,
}

impl ProbeHintKind {
    /// This kind's stable ABI code, as it appears in `PluginProbeHint::kind`.
    #[must_use]
    #[inline]
    pub const fn to_abi(self) -> u32 {
        match self {
            Self::None => 0,
            Self::UsbVidPid => 1,
            Self::HidVidPid => 2,
            Self::DmiMatch => 3,
        }
    }

    /// Parses a raw ABI code read out of foreign plugin memory, returning
    /// `None` for a code no variant covers. See [`PluginBus::from_abi`] for why
    /// the daemon validates the raw integer instead of reading the enum
    /// directly.
    #[must_use]
    #[inline]
    pub const fn from_abi(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::None),
            1 => Some(Self::UsbVidPid),
            2 => Some(Self::HidVidPid),
            3 => Some(Self::DmiMatch),
            _ => None,
        }
    }
}

/// Severity of one `PluginLogFn` call, mirroring `tracing`'s levels.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginLogLevel {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
    Trace = 4,
}

impl PluginLogLevel {
    /// This level's stable ABI byte, as passed across `PluginLogFn` /
    /// `PluginInitFn`.
    #[must_use]
    #[inline]
    pub const fn to_abi(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
            Self::Info => 2,
            Self::Debug => 3,
            Self::Trace => 4,
        }
    }

    /// Parses an untrusted raw ABI byte without materializing an invalid enum.
    #[must_use]
    #[inline]
    pub const fn from_abi(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Error),
            1 => Some(Self::Warn),
            2 => Some(Self::Info),
            3 => Some(Self::Debug),
            4 => Some(Self::Trace),
            _ => None,
        }
    }
}

/// Decodes an ABI boolean flag. The plugin ABI carries every plugin-produced
/// boolean (such as a `probe` or callback-envelope return value) as a `u8`
/// rather than a Rust `bool`, because a foreign plugin can write any byte
/// and materializing a `bool` from a byte other than `0` or `1` is undefined
/// behavior. Any nonzero byte means true, matching C's convention, so this
/// decode is total for every possible byte.
#[must_use]
#[inline]
pub const fn abi_bool(byte: u8) -> bool {
    byte != 0
}

/// Result of checking whether a plugin can operate on this system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The platform or configured prerequisites are not supported.
    Unsupported,
    /// The plugin is supported but no usable hardware is currently present.
    Dormant,
    /// The plugin is supported and its prerequisites or hardware are present.
    Ready,
}

impl ProbeOutcome {
    /// Returns this outcome's native ABI byte.
    #[must_use]
    #[inline]
    pub const fn to_abi(self) -> u8 {
        match self {
            Self::Unsupported => 0,
            Self::Dormant => 1,
            Self::Ready => 2,
        }
    }

    /// Decodes an untrusted native ABI byte.
    #[must_use]
    #[inline]
    pub const fn from_abi(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Unsupported),
            1 => Some(Self::Dormant),
            2 => Some(Self::Ready),
            _ => None,
        }
    }
}

/// Why the daemon is asking a plugin to re-enumerate its hardware.
///
/// A rescan is advisory context, not a different operation: every reason asks
/// for the same thing, a freshly enumerated view rather than whatever a
/// discovery cache last recorded. The reason exists so a plugin can be
/// proportionate about it: a resume justifies dropping cached handles and
/// re-probing from scratch, while an ordinary device-change notification may
/// only warrant re-reading the bus.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanReason {
    /// The system resumed from suspend. Hardware may have re-enumerated under
    /// different kernel device nodes, and any cached handle or path a plugin
    /// held across the suspend should be treated as stale.
    Resume = 0,
    /// The host reported a device arriving or departing.
    DeviceChange = 1,
    /// An operator asked for a rescan explicitly, over the control protocol or
    /// by signalling the daemon.
    Operator = 2,
}

impl RescanReason {
    /// This reason's stable ABI byte, as passed across `PluginRescanFn`.
    #[must_use]
    #[inline]
    pub const fn to_abi(self) -> u8 {
        match self {
            Self::Resume => 0,
            Self::DeviceChange => 1,
            Self::Operator => 2,
        }
    }

    /// Decodes an untrusted ABI byte received across the FFI boundary,
    /// returning `None` for a byte no variant covers. See
    /// [`PluginBus::from_abi`] for why both sides exchange the raw byte and
    /// validate with this rather than passing the enum directly.
    #[must_use]
    #[inline]
    pub const fn from_abi(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Resume),
            1 => Some(Self::DeviceChange),
            2 => Some(Self::Operator),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "abi_tests.rs"]
mod tests;

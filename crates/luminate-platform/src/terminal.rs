// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Terminal-safe rendering for process output and diagnostic sinks.

use std::borrow::Cow;
use std::fmt;

use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::{DefaultFields, FormatFields, Writer};

/// Escapes every Unicode control character while preserving ordinary text.
///
/// This covers C0, DEL, and C1, including every control byte used to introduce
/// ECMA-48/ANSI terminal sequences. The escaped form is intended for display,
/// not for round-tripping as application data.
#[must_use]
pub fn escape(input: &str) -> Cow<'_, str> {
    if !input.chars().any(char::is_control) {
        return Cow::Borrowed(input);
    }

    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        if character.is_control() {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
    }
    Cow::Owned(escaped)
}

/// Escapes raw terminal controls in an already serialized JSON document.
///
/// JSON's structural line feeds are retained. Any control in a JSON string is
/// written as a JSON Unicode escape, so parsing the result produces exactly the
/// original value. `serde_json` already escapes C0; this function additionally
/// covers DEL and C1, which JSON permits as raw Unicode characters.
#[must_use]
pub fn escape_json(serialized: &str) -> Cow<'_, str> {
    if !serialized
        .chars()
        .any(|character| character != '\n' && character.is_control())
    {
        return Cow::Borrowed(serialized);
    }

    let mut escaped = String::with_capacity(serialized.len());
    for character in serialized.chars() {
        if character == '\n' {
            escaped.push(character);
        } else if character.is_control() {
            use std::fmt::Write as _;
            let _ = write!(escaped, "\\u{:04x}", u32::from(character));
        } else {
            escaped.push(character);
        }
    }
    Cow::Owned(escaped)
}

/// A tracing field formatter that escapes controls in event and span values.
///
/// Formatting metadata and optional ANSI styling remain owned by
/// `tracing-subscriber`; only field values pass through the escaping boundary.
/// This preserves intentional formatter styling without allowing a value to
/// introduce its own terminal sequence or forged line.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerminalSafeFields;

impl<'writer> FormatFields<'writer> for TerminalSafeFields {
    fn format_fields<R: RecordFields>(
        &self,
        mut writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        let mut rendered = String::new();
        DefaultFields::new().format_fields(Writer::new(&mut rendered), fields)?;
        writer.write_str(&escape(&rendered))
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;

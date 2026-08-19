// SPDX-License-Identifier: LGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Cryptographically secure random bytes from the local platform.

use std::io;

/// Fills `buf` with cryptographically secure random bytes from the local
/// platform's RNG.
///
/// This is meant for unpredictability (for example, a temporary filename an
/// attacker should not be able to pre-guess), not for cryptographic key
/// material; callers with key-generation needs should reach for a vetted
/// crate instead.
///
/// # Errors
///
/// Returns an error if the platform RNG cannot be reached.
pub fn fill_bytes(buf: &mut [u8]) -> io::Result<()> {
    getrandom::fill(buf).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::fill_bytes;

    #[test]
    fn fills_the_whole_buffer() {
        let mut buf = [0_u8; 32];
        fill_bytes(&mut buf).expect("draw from the system RNG");
        assert!(buf.iter().any(|&byte| byte != 0), "buffer was left zeroed");
    }
}

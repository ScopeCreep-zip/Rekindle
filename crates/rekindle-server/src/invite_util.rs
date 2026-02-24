//! Invite code generation utilities shared between IPC and RPC paths.

use rand::RngCore;

/// Generate a random invite code (base32-encoded, ~20 chars, 96 bits entropy).
///
/// Uses `OsRng` for cryptographic randomness. The resulting string is lowercase
/// alphanumeric (a-z, 2-7), safe for URLs and case-insensitive matching.
pub fn generate_invite_code() -> String {
    let mut bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

/// Encode bytes as a lowercase base32 string (RFC 4648, no padding).
fn base32_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut result = String::new();
    let mut buffer: u64 = 0;
    let mut bits_left = 0;

    for &byte in bytes {
        buffer = (buffer << 8) | u64::from(byte);
        bits_left += 8;
        while bits_left >= 5 {
            bits_left -= 5;
            let index = ((buffer >> bits_left) & 0x1F) as usize;
            result.push(ALPHABET[index] as char);
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1F) as usize;
        result.push(ALPHABET[index] as char);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_code_is_consistent_length() {
        let code = generate_invite_code();
        // 12 bytes * 8 bits / 5 bits per char = 19.2 → 20 chars (ceil)
        assert!(
            code.len() >= 19 && code.len() <= 20,
            "got len={}",
            code.len()
        );
    }

    #[test]
    fn invite_code_is_lowercase_base32() {
        let code = generate_invite_code();
        assert!(code
            .chars()
            .all(|c| c.is_ascii_lowercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn two_codes_are_different() {
        let a = generate_invite_code();
        let b = generate_invite_code();
        assert_ne!(a, b);
    }
}

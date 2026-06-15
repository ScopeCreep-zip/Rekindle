//! Input validation at the dispatch boundary.
//!
//! Validates before any crypto or I/O — fail fast on bad input.
//! Called from `service/delegate/messaging.rs` at the top of every
//! public-facing method.

use crate::ChatError;

const MAX_MESSAGE_BYTES: usize = 65_536;
const MIN_KEY_HEX_LEN: usize = 16;

/// Validate a DHT key or public key hex string.
pub fn validate_key(key: &str, label: &str) -> Result<(), ChatError> {
    if key.is_empty() {
        return Err(ChatError::Internal(format!("{label} is empty")));
    }
    if key.len() < MIN_KEY_HEX_LEN {
        return Err(ChatError::Internal(format!(
            "{label} too short: {} chars (min {MIN_KEY_HEX_LEN})",
            key.len()
        )));
    }
    // Allow hex chars + VLD0: prefix colon
    if !key.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
        return Err(ChatError::Internal(format!(
            "{label} contains non-hex characters"
        )));
    }
    Ok(())
}

/// Validate a message body.
pub fn validate_message_body(body: &str) -> Result<(), ChatError> {
    if body.is_empty() {
        return Err(ChatError::Internal("message body is empty".into()));
    }
    if body.len() > MAX_MESSAGE_BYTES {
        return Err(ChatError::MessageTooLarge {
            len: body.len(),
            max: MAX_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_key_rejected() {
        assert!(validate_key("", "test").is_err());
    }

    #[test]
    fn short_key_rejected() {
        assert!(validate_key("abcdef", "test").is_err());
    }

    #[test]
    fn valid_hex_key_accepted() {
        assert!(validate_key("abcdef0123456789", "test").is_ok());
    }

    #[test]
    fn vld0_prefix_accepted() {
        assert!(validate_key("VLD0:abcdef0123456789", "test").is_ok());
    }

    #[test]
    fn non_hex_rejected() {
        assert!(validate_key("not_a_valid_hex_key!", "test").is_err());
    }

    #[test]
    fn empty_body_rejected() {
        assert!(validate_message_body("").is_err());
    }

    #[test]
    fn oversized_body_rejected() {
        let big = "x".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(validate_message_body(&big).is_err());
    }

    #[test]
    fn normal_body_accepted() {
        assert!(validate_message_body("hello world").is_ok());
    }
}

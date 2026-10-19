pub mod account;
pub mod community;
pub mod conversation;
pub mod friends;
pub mod mailbox;
pub mod pool;
pub mod profile;
pub mod route_imports;

use crate::error::ProtocolError;

/// Parse a DHT record key string into a Veilid `RecordKey`.
///
/// # Errors
/// The string is not a record key.
pub fn parse_record_key(key: &str) -> Result<veilid_core::RecordKey, ProtocolError> {
    key.parse()
        .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    /// The Veilid-free `record_key_fingerprint` (plan U1.9) reads the same
    /// opaque key bytes Veilid's own parser does.
    #[test]
    fn the_record_key_fingerprint_matches_veilids_decoding() {
        let bytes: Vec<u8> = (1..=32).collect();
        let text = format!(
            "VLD0:{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
        );
        let parsed = super::parse_record_key(&text).expect("a record key");
        let opaque: &[u8] = parsed.ref_value().ref_key().as_ref();
        assert_eq!(opaque, bytes.as_slice());
        assert_eq!(
            rekindle_crypto::identity::record_key_fingerprint(&text),
            Some(rekindle_crypto::identity::key_fingerprint(opaque))
        );
    }
}

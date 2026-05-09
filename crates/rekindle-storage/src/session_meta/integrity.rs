//! Session.json tamper detection — BLAKE3 keyed MAC.

/// Compute MAC over session metadata bytes.
pub fn compute_mac(key: &[u8; 32], data: &[u8]) -> String {
    let mac = blake3::keyed_hash(key, data);
    hex::encode(mac.as_bytes())
}

/// Split session.json content into (json_portion, mac_hex).
pub fn split_mac(content: &str) -> Option<(&str, String)> {
    let mut parts = content.splitn(2, "\n---MAC---\n");
    let json = parts.next()?;
    let mac = parts.next()?.trim().to_string();
    if mac.is_empty() {
        return None;
    }
    Some((json, mac))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_mac() {
        let key = [42u8; 32];
        let data = b"test session json";
        let mac = compute_mac(&key, data);
        assert_eq!(mac.len(), 64); // BLAKE3 = 32 bytes = 64 hex chars

        let content = format!("test session json\n---MAC---\n{mac}");
        let (json, stored) = split_mac(&content).unwrap();
        assert_eq!(json, "test session json");
        assert_eq!(stored, mac);
    }

    #[test]
    fn tampered_content_detected() {
        let key = [42u8; 32];
        let mac = compute_mac(&key, b"original");
        let tampered_mac = compute_mac(&key, b"tampered");
        assert_ne!(mac, tampered_mac);
    }

    #[test]
    fn missing_separator_returns_none() {
        assert!(split_mac("no separator here").is_none());
    }
}

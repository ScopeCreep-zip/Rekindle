//! Architecture §6 — channel MEK delivery shape used inside
//! `BootstrapResponse.channel_meks`. The MEK itself is wrapped (sealed
//! to the recipient's pseudonym key) so this struct carries opaque
//! ciphertext, not raw key material.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMekDelivery {
    /// 16-byte UUID hex of the channel this MEK belongs to. None when
    /// the delivery is the community-wide MEK (architecture §6.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Architecture §6.7 — monotonically increasing generation counter
    /// the rotator bumps on every MEK rotation.
    pub generation: u64,
    /// Wrapped MEK ciphertext (sealed to the recipient's pseudonym
    /// public key via the deterministic ECDH wrapper).
    pub wrapped_mek: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_mek_delivery_roundtrip() {
        let d = ChannelMekDelivery {
            channel_id: Some("ch_01".into()),
            generation: 7,
            wrapped_mek: vec![0xde, 0xad, 0xbe, 0xef],
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: ChannelMekDelivery = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn community_wide_mek_omits_channel() {
        let d = ChannelMekDelivery {
            channel_id: None,
            generation: 1,
            wrapped_mek: vec![1],
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("channelId"));
    }
}

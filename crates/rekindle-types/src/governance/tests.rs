//! Tests for [`super::GovernanceEntry`].

use crate::id::ChannelId;

use super::*;

#[test]
fn governance_entry_serde_roundtrip() {
    let entry = GovernanceEntry::ChannelCreated {
        channel_id: ChannelId([1u8; 16]),
        name: "general".into(),
        channel_type: "text".into(),
        record_key: "VLD0:abc123".into(),
        category_id: None,
        position: 0,
        parent_voice_channel_id: None,
    };
    let json = serde_json::to_string(&entry).unwrap();
    let back: GovernanceEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(entry, back);
}

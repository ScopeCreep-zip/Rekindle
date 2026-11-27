use super::*;

// `ChannelKind`'s own tests moved to `rekindle_types::channel` with
// the type itself; this crate only re-exports it now.

#[test]
fn community_metadata_v2_serde() {
    let meta = CommunityMetadataV2 {
        name: "Test".into(),
        description: Some("desc".into()),
        icon_hash: None,
        created_at: 1_234_567_890,
        owner_pseudonym: "abc".into(),
        last_refreshed: 0,
    };
    let json = serde_json::to_string(&meta).unwrap();
    assert!(json.contains("ownerPseudonym"));
    let back: CommunityMetadataV2 = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "Test");
}

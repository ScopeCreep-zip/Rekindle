use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};
use serde_json::{json, Value};

pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "community.profilePatch",
        serde_json::to_value(CommunityProfilePatch {
            bio: ProfileField::Set("At the LAN".into()),
            theme_color: ProfileField::Clear,
            avatar_ref: ProfileField::Set("a".repeat(64)),
            banner_ref: ProfileField::Clear,
            ..CommunityProfilePatch::default()
        })
        .unwrap(),
    );
}

#[test]
fn profile_patch_requires_typed_operations() {
    assert!(serde_json::from_value::<CommunityProfilePatch>(json!({})).is_err());
    assert!(serde_json::from_value::<ProfileField<String>>(json!("")).is_err());
    assert!(serde_json::from_value::<ProfileField<String>>(json!({"op":"set"})).is_err());
    assert!(serde_json::from_value::<ProfileField<String>>(json!({"op":"keep"})).is_err());
    let edit: ProfileField<String> = serde_json::from_value(json!({"op":"clear"})).unwrap();
    assert_eq!(edit, ProfileField::Clear);
}

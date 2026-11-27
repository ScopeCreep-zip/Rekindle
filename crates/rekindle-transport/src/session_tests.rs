use super::*;

fn test_identity() -> SessionIdentity {
    SessionIdentity {
        public_key_hex: "abcd1234".repeat(8),
        display_name: "alice".into(),
        profile_dht_key: "VLD0:profile:key".into(),
        mailbox_dht_key: "VLD0:mailbox:key".into(),
        friend_list_dht_key: "VLD0:friends:key".into(),
        profile_owner_keypair: None,
        friend_list_owner_keypair: None,
    }
}

fn test_membership() -> CommunityMembership {
    CommunityMembership {
        anchor: "VLD0:gov:key".into(),
        pseudonym_key: "pseudonym_hex".into(),
        registry_key: "VLD0:reg:key".into(),
        slot_index: 3,
        community_name: "dev-team".into(),
        slot_seed: None,
        segment_index: Some(0),
        mek_generation: 2,
    }
}

#[test]
fn community_by_name_case_insensitive() {
    let mut session = Session::new(test_identity());
    session.join_community(test_membership());

    assert!(session.community_by_name("dev-team").is_some());
    assert!(session.community_by_name("Dev-Team").is_some());
    assert!(session.community_by_name("DEV-TEAM").is_some());
    assert!(session.community_by_name("nonexistent").is_none());
}

#[test]
fn community_by_name_ambiguous_returns_none() {
    let mut session = Session::new(test_identity());

    let mut m1 = test_membership();
    m1.anchor = "gov1".into();
    m1.community_name = "shared-name".into();
    session.join_community(m1);

    let mut m2 = test_membership();
    m2.anchor = "gov2".into();
    m2.community_name = "shared-name".into();
    session.join_community(m2);

    // Two communities with the same name — ambiguous
    assert!(session.community_by_name("shared-name").is_none());
}

#[test]
fn leave_community_removes() {
    let mut session = Session::new(test_identity());
    session.join_community(test_membership());
    assert_eq!(session.communities.len(), 1);

    session.leave_community("VLD0:gov:key");
    assert_eq!(session.communities.len(), 0);
}

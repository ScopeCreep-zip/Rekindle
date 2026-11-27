use super::*;

fn every_variant() -> Vec<VaultKey> {
    let c = || "c_1:x".to_owned();
    vec![
        VaultKey::IdentityEd25519,
        VaultKey::ProfileOwnerKeypair,
        VaultKey::FriendListOwnerKeypair,
        VaultKey::AccountOwnerKeypair,
        VaultKey::PersonalSyncOwnerKeypair,
        VaultKey::PrekeyRecordOwnerKeypair,
        VaultKey::PendingPrekeyOwner,
        VaultKey::PendingRotation,
        VaultKey::CommunityMek { community: c() },
        VaultKey::CommunityMekGeneration {
            community: c(),
            generation: 5,
        },
        VaultKey::CommunityMekGenerationsIndex { community: c() },
        VaultKey::ChannelMek {
            community: c(),
            channel: "ch_2".into(),
        },
        VaultKey::ChannelMekGeneration {
            community: c(),
            channel: "ch".into(),
            generation: 5,
        },
        VaultKey::ChannelMekGenerationsIndex {
            community: c(),
            channel: "ch".into(),
        },
        VaultKey::SlotKeypair { community: c() },
        VaultKey::RequestInboxOwnerKeypair { record: c() },
        VaultKey::DmParticipantOwner { conversation: c() },
        VaultKey::GroupRendezvousWriters { group: c() },
        VaultKey::RegistryKeypair { community: c() },
        VaultKey::SlotSeed { community: c() },
        VaultKey::MemberRecordOwner { community: c() },
        VaultKey::MemberHpkeSecret { community: c() },
        VaultKey::DeviceVaultSecret,
        VaultKey::DeviceOpKey { community: c() },
        VaultKey::AuditMacKey,
        VaultKey::AuditTail,
        VaultKey::ClientKey {
            public_key: "ab".repeat(32),
        },
    ]
}

/// Every variant's key parses back to it, in its namespace only, even
/// when a field holds the separator.
#[test]
fn every_key_parses_back_to_its_variant() {
    for vk in every_variant() {
        let key = vk.key();
        assert_eq!(
            VaultKey::parse(vk.namespace(), &key),
            Some(vk.clone()),
            "{key}"
        );
        assert_eq!(VaultKey::parse("elsewhere", &key), None);
    }
    assert_eq!(
        VaultKey::ChannelMek {
            community: "a".into(),
            channel: "b".into()
        }
        .key(),
        "channel_mek:1:a:1:b"
    );
    assert_eq!(
        VaultKey::parse("communities", "community_mek:2:7"),
        None,
        "length mismatch"
    );
    assert_eq!(VaultKey::parse("communities", "community_mek:x:7"), None);
}

/// Field values cannot alias: two different (community, channel) pairs
/// whose naive concatenation agrees get different keys.
#[test]
fn fields_do_not_alias() {
    let a = VaultKey::ChannelMek {
        community: "a_b".into(),
        channel: "c".into(),
    };
    let b = VaultKey::ChannelMek {
        community: "a".into(),
        channel: "b_c".into(),
    };
    assert_ne!(a.key(), b.key());
}

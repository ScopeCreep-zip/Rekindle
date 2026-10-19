//! Typed vault keys — the storage security boundary.
//!
//! Every value persisted to a [`crate::VaultStore`] is addressed by a
//! `VaultKey` variant rather than a raw `(namespace, key)` string pair. This
//! makes the set of storable secrets a closed, auditable enum: there is
//! deliberately no `Generic { namespace, key }` escape hatch, so a typo or an
//! attacker-influenced string can never address an arbitrary slot.
//!
//! Each variant maps to a `(namespace, key)` pair via
//! [`VaultKey::namespace`] and [`VaultKey::key`], and back via
//! [`VaultKey::parse`]. A key is its variant's tag followed by each field
//! as `:<byte length>:<value>` (plan D1), so no field value can be read as
//! another field and every stored key parses back to one variant.

use std::borrow::Cow;

/// A typed address for one entry in a [`crate::VaultStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultKey {
    /// Ed25519 signing private key — `("identity", "ed25519_private")`.
    IdentityEd25519,

    /// Community-level MEK — `("communities", "mek_{community}")`.
    CommunityMek {
        community: String,
    },
    /// Community-level MEK at a specific generation, kept so history
    /// written under a replaced key stays readable —
    /// `("communities", "community_mek_{community}_{generation}")`.
    CommunityMekGeneration {
        community: String,
        generation: u64,
    },
    /// Community-level MEK generations index —
    /// `("communities", "community_mek_generations_{community}")`.
    CommunityMekGenerationsIndex {
        community: String,
    },
    /// Per-channel latest MEK —
    /// `("communities", "mek_{community}_{channel}")`.
    ChannelMek {
        community: String,
        channel: String,
    },
    /// Per-channel MEK at a specific generation —
    /// `("communities", "mek_{community}_{channel}_{generation}")`.
    ChannelMekGeneration {
        community: String,
        channel: String,
        generation: u64,
    },
    /// Per-channel generations index —
    /// `("communities", "mek_generations_{community}_{channel}")`.
    ChannelMekGenerationsIndex {
        community: String,
        channel: String,
    },
    /// SMPL slot keypair — `("communities", "slot_keypair_{community}")`.
    SlotKeypair {
        community: String,
    },
    /// SMPL registry owner keypair —
    /// `("communities", "registry_keypair_{community}")`.
    RegistryKeypair {
        community: String,
    },
    /// SMPL slot seed (hex) — `("communities", "slot_seed_{community}")`.
    SlotSeed {
        community: String,
    },

    /// Audit chain MAC key — `("audit", "mac_key")`.
    AuditMacKey,
    /// Audit chain tail anchor — `("audit", "tail")`.
    AuditTail,

    /// The identity's DHT records' owner keypairs (plan D1, moved from the
    /// `identity` table): profile, friend list, account, personal sync.
    ProfileOwnerKeypair,
    FriendListOwnerKeypair,
    AccountOwnerKeypair,
    PersonalSyncOwnerKeypair,

    /// A bus client's registered static public key (hex) and its clearance
    /// (plan D1) — `("clients", "client:<len>:<key>")`.
    ClientKey {
        public_key: String,
    },
}

impl VaultKey {
    /// The storage namespace — the first column of the `entries` table.
    #[must_use]
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::IdentityEd25519
            | Self::ProfileOwnerKeypair
            | Self::FriendListOwnerKeypair
            | Self::AccountOwnerKeypair
            | Self::PersonalSyncOwnerKeypair => "identity",
            Self::CommunityMek { .. }
            | Self::CommunityMekGeneration { .. }
            | Self::CommunityMekGenerationsIndex { .. }
            | Self::ChannelMek { .. }
            | Self::ChannelMekGeneration { .. }
            | Self::ChannelMekGenerationsIndex { .. }
            | Self::SlotKeypair { .. }
            | Self::RegistryKeypair { .. }
            | Self::SlotSeed { .. } => "communities",
            Self::AuditMacKey | Self::AuditTail => "audit",
            Self::ClientKey { .. } => "clients",
        }
    }

    /// The variant's tag and its fields, in order.
    fn parts(&self) -> (&'static str, Vec<Cow<'_, str>>) {
        let s = |v: &str| Cow::Owned(v.to_owned());
        let n = |v: u64| Cow::Owned(v.to_string());
        match self {
            Self::IdentityEd25519 => ("ed25519_private", vec![]),
            Self::ProfileOwnerKeypair => ("profile_owner_keypair", vec![]),
            Self::FriendListOwnerKeypair => ("friend_list_owner_keypair", vec![]),
            Self::AccountOwnerKeypair => ("account_owner_keypair", vec![]),
            Self::PersonalSyncOwnerKeypair => ("personal_sync_owner_keypair", vec![]),
            Self::CommunityMek { community } => ("community_mek", vec![s(community)]),
            Self::CommunityMekGeneration {
                community,
                generation,
            } => (
                "community_mek_generation",
                vec![s(community), n(*generation)],
            ),
            Self::CommunityMekGenerationsIndex { community } => {
                ("community_mek_generations", vec![s(community)])
            }
            Self::ChannelMek { community, channel } => {
                ("channel_mek", vec![s(community), s(channel)])
            }
            Self::ChannelMekGeneration {
                community,
                channel,
                generation,
            } => (
                "channel_mek_generation",
                vec![s(community), s(channel), n(*generation)],
            ),
            Self::ChannelMekGenerationsIndex { community, channel } => {
                ("channel_mek_generations", vec![s(community), s(channel)])
            }
            Self::SlotKeypair { community } => ("slot_keypair", vec![s(community)]),
            Self::RegistryKeypair { community } => ("registry_keypair", vec![s(community)]),
            Self::SlotSeed { community } => ("slot_seed", vec![s(community)]),
            Self::AuditMacKey => ("mac_key", vec![]),
            Self::AuditTail => ("tail", vec![]),
            Self::ClientKey { public_key } => ("client", vec![s(public_key)]),
        }
    }

    /// The storage key — the second column of the `entries` table: the
    /// tag, then each field as `:<byte length>:<value>`.
    #[must_use]
    pub fn key(&self) -> Cow<'static, str> {
        let (tag, fields) = self.parts();
        if fields.is_empty() {
            return Cow::Borrowed(tag);
        }
        let mut out = tag.to_owned();
        for field in fields {
            out.push(':');
            out.push_str(&field.len().to_string());
            out.push(':');
            out.push_str(&field);
        }
        Cow::Owned(out)
    }

    /// The variant stored under `(namespace, key)`, or `None` when the pair
    /// names none.
    #[must_use]
    pub fn parse(namespace: &str, key: &str) -> Option<Self> {
        let (tag, fields) = split_fields(key)?;
        let parsed = match (tag, fields.as_slice()) {
            ("ed25519_private", []) => Self::IdentityEd25519,
            ("profile_owner_keypair", []) => Self::ProfileOwnerKeypair,
            ("friend_list_owner_keypair", []) => Self::FriendListOwnerKeypair,
            ("account_owner_keypair", []) => Self::AccountOwnerKeypair,
            ("personal_sync_owner_keypair", []) => Self::PersonalSyncOwnerKeypair,
            ("community_mek", [c]) => Self::CommunityMek {
                community: c.clone(),
            },
            ("community_mek_generation", [c, g]) => Self::CommunityMekGeneration {
                community: c.clone(),
                generation: g.parse().ok()?,
            },
            ("community_mek_generations", [c]) => Self::CommunityMekGenerationsIndex {
                community: c.clone(),
            },
            ("channel_mek", [c, ch]) => Self::ChannelMek {
                community: c.clone(),
                channel: ch.clone(),
            },
            ("channel_mek_generation", [c, ch, g]) => Self::ChannelMekGeneration {
                community: c.clone(),
                channel: ch.clone(),
                generation: g.parse().ok()?,
            },
            ("channel_mek_generations", [c, ch]) => Self::ChannelMekGenerationsIndex {
                community: c.clone(),
                channel: ch.clone(),
            },
            ("slot_keypair", [c]) => Self::SlotKeypair {
                community: c.clone(),
            },
            ("registry_keypair", [c]) => Self::RegistryKeypair {
                community: c.clone(),
            },
            ("slot_seed", [c]) => Self::SlotSeed {
                community: c.clone(),
            },
            ("mac_key", []) => Self::AuditMacKey,
            ("tail", []) => Self::AuditTail,
            ("client", [k]) => Self::ClientKey {
                public_key: k.clone(),
            },
            _ => return None,
        };
        (parsed.namespace() == namespace).then_some(parsed)
    }
}

/// A key's tag and its length-prefixed fields.
fn split_fields(key: &str) -> Option<(&str, Vec<String>)> {
    let Some((tag, mut rest)) = key.split_once(':') else {
        return Some((key, Vec::new()));
    };
    let mut fields = Vec::new();
    loop {
        let (len, after) = rest.split_once(':')?;
        let len: usize = len.parse().ok()?;
        let value = after.get(..len)?;
        fields.push(value.to_owned());
        let tail = &after[len..];
        if tail.is_empty() {
            return Some((tag, fields));
        }
        rest = tail.strip_prefix(':')?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<VaultKey> {
        let c = || "c_1:x".to_owned();
        vec![
            VaultKey::IdentityEd25519,
            VaultKey::ProfileOwnerKeypair,
            VaultKey::FriendListOwnerKeypair,
            VaultKey::AccountOwnerKeypair,
            VaultKey::PersonalSyncOwnerKeypair,
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
            VaultKey::RegistryKeypair { community: c() },
            VaultKey::SlotSeed { community: c() },
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
}

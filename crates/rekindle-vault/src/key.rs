//! Typed vault keys — the storage security boundary.
//!
//! Every value persisted to a [`crate::VaultStore`] is addressed by a
//! `VaultKey` variant rather than a raw `(namespace, key)` string pair. This
//! makes the set of storable secrets a closed, auditable enum: there is
//! deliberately no `Generic { namespace, key }` escape hatch, so a typo or an
//! attacker-influenced string can never address an arbitrary slot.
//!
//! Each variant maps to a fixed `(namespace, key)` pair via
//! [`VaultKey::namespace`] and [`VaultKey::key`]. Those strings are the
//! on-disk identifiers; they must stay byte-stable or existing vaults stop
//! resolving their entries.

use std::borrow::Cow;

/// A typed address for one entry in a [`crate::VaultStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultKey {
    /// Ed25519 signing private key — `("identity", "ed25519_private")`.
    IdentityEd25519,

    /// Signal identity keypair blob — `("signal", "identity_keypair")`.
    SignalIdentity,
    /// Signal registration id — `("signal", "registration_id")`.
    SignalRegistrationId,
    /// Per-peer trusted identity (trust-on-first-use) —
    /// `("signal", "trusted:{peer}")`.
    SignalTrusted { peer: String },
    /// Per-peer Signal session — `("signal", "session:{peer}")`.
    SignalSession { peer: String },
    /// Index of peers that have a stored session —
    /// `("signal", "session_index")`.
    SignalSessionIndex,
    /// One-time prekey by id — `("signal", "prekey:{id}")`.
    SignalPrekey { id: u32 },
    /// Index of stored one-time prekey ids — `("signal", "prekey_index")`.
    SignalPrekeyIndex,
    /// Signed prekey by id — `("signal", "signed_prekey:{id}")`.
    SignalSignedPrekey { id: u32 },
    /// PQXDH last-resort ML-KEM-768 secret — `("signal", "pq_lr:{id}")`.
    SignalPqLastResort { id: u32 },
    /// PQXDH one-time ML-KEM-768 secret — `("signal", "pq_ot:{id}")`.
    SignalPqOneTime { id: u32 },

    /// Community-level MEK — `("communities", "mek_{community}")`.
    CommunityMek { community: String },
    /// Per-channel latest MEK —
    /// `("communities", "mek_{community}_{channel}")`.
    ChannelMek { community: String, channel: String },
    /// Per-channel MEK at a specific generation —
    /// `("communities", "mek_{community}_{channel}_{generation}")`.
    ChannelMekGeneration {
        community: String,
        channel: String,
        generation: u64,
    },
    /// Per-channel generations index —
    /// `("communities", "mek_generations_{community}_{channel}")`.
    ChannelMekGenerationsIndex { community: String, channel: String },
    /// SMPL slot keypair — `("communities", "slot_keypair_{community}")`.
    SlotKeypair { community: String },
    /// SMPL registry owner keypair —
    /// `("communities", "registry_keypair_{community}")`.
    RegistryKeypair { community: String },
    /// SMPL slot seed (hex) — `("communities", "slot_seed_{community}")`.
    SlotSeed { community: String },

    /// Audit chain MAC key — `("audit", "mac_key")`.
    AuditMacKey,
    /// Audit chain tail anchor — `("audit", "tail")`.
    AuditTail,
}

impl VaultKey {
    /// The storage namespace — the first column of the `entries` table.
    #[must_use]
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::IdentityEd25519 => "identity",
            Self::SignalIdentity
            | Self::SignalRegistrationId
            | Self::SignalTrusted { .. }
            | Self::SignalSession { .. }
            | Self::SignalSessionIndex
            | Self::SignalPrekey { .. }
            | Self::SignalPrekeyIndex
            | Self::SignalSignedPrekey { .. }
            | Self::SignalPqLastResort { .. }
            | Self::SignalPqOneTime { .. } => "signal",
            Self::CommunityMek { .. }
            | Self::ChannelMek { .. }
            | Self::ChannelMekGeneration { .. }
            | Self::ChannelMekGenerationsIndex { .. }
            | Self::SlotKeypair { .. }
            | Self::RegistryKeypair { .. }
            | Self::SlotSeed { .. } => "communities",
            Self::AuditMacKey | Self::AuditTail => "audit",
        }
    }

    /// The storage key — the second column of the `entries` table. Static
    /// variants borrow a `&'static str`; parameterized variants format an
    /// owned string from their fields.
    #[must_use]
    pub fn key(&self) -> Cow<'static, str> {
        match self {
            Self::IdentityEd25519 => Cow::Borrowed("ed25519_private"),
            Self::SignalIdentity => Cow::Borrowed("identity_keypair"),
            Self::SignalRegistrationId => Cow::Borrowed("registration_id"),
            Self::SignalTrusted { peer } => Cow::Owned(format!("trusted:{peer}")),
            Self::SignalSession { peer } => Cow::Owned(format!("session:{peer}")),
            Self::SignalSessionIndex => Cow::Borrowed("session_index"),
            Self::SignalPrekey { id } => Cow::Owned(format!("prekey:{id}")),
            Self::SignalPrekeyIndex => Cow::Borrowed("prekey_index"),
            Self::SignalSignedPrekey { id } => Cow::Owned(format!("signed_prekey:{id}")),
            Self::SignalPqLastResort { id } => Cow::Owned(format!("pq_lr:{id}")),
            Self::SignalPqOneTime { id } => Cow::Owned(format!("pq_ot:{id}")),
            Self::CommunityMek { community } => Cow::Owned(format!("mek_{community}")),
            Self::ChannelMek { community, channel } => {
                Cow::Owned(format!("mek_{community}_{channel}"))
            }
            Self::ChannelMekGeneration {
                community,
                channel,
                generation,
            } => Cow::Owned(format!("mek_{community}_{channel}_{generation}")),
            Self::ChannelMekGenerationsIndex { community, channel } => {
                Cow::Owned(format!("mek_generations_{community}_{channel}"))
            }
            Self::SlotKeypair { community } => Cow::Owned(format!("slot_keypair_{community}")),
            Self::RegistryKeypair { community } => {
                Cow::Owned(format!("registry_keypair_{community}"))
            }
            Self::SlotSeed { community } => Cow::Owned(format!("slot_seed_{community}")),
            Self::AuditMacKey => Cow::Borrowed("mac_key"),
            Self::AuditTail => Cow::Borrowed("tail"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard: every variant must map to the exact `(namespace,
    /// key)` strings the pre-typed stringly API produced, or existing vaults
    /// silently stop resolving their entries.
    #[test]
    fn namespace_and_key_strings_are_byte_stable() {
        let cases: &[(VaultKey, &str, &str)] = &[
            (VaultKey::IdentityEd25519, "identity", "ed25519_private"),
            (VaultKey::SignalIdentity, "signal", "identity_keypair"),
            (VaultKey::SignalRegistrationId, "signal", "registration_id"),
            (
                VaultKey::SignalTrusted {
                    peer: "peer-abc".into(),
                },
                "signal",
                "trusted:peer-abc",
            ),
            (
                VaultKey::SignalSession {
                    peer: "peer-abc".into(),
                },
                "signal",
                "session:peer-abc",
            ),
            (VaultKey::SignalSessionIndex, "signal", "session_index"),
            (VaultKey::SignalPrekey { id: 7 }, "signal", "prekey:7"),
            (VaultKey::SignalPrekeyIndex, "signal", "prekey_index"),
            (
                VaultKey::SignalSignedPrekey { id: 9 },
                "signal",
                "signed_prekey:9",
            ),
            (VaultKey::SignalPqLastResort { id: 3 }, "signal", "pq_lr:3"),
            (VaultKey::SignalPqOneTime { id: 4 }, "signal", "pq_ot:4"),
            (
                VaultKey::CommunityMek {
                    community: "c1".into(),
                },
                "communities",
                "mek_c1",
            ),
            (
                VaultKey::ChannelMek {
                    community: "c1".into(),
                    channel: "ch".into(),
                },
                "communities",
                "mek_c1_ch",
            ),
            (
                VaultKey::ChannelMekGeneration {
                    community: "c1".into(),
                    channel: "ch".into(),
                    generation: 5,
                },
                "communities",
                "mek_c1_ch_5",
            ),
            (
                VaultKey::ChannelMekGenerationsIndex {
                    community: "c1".into(),
                    channel: "ch".into(),
                },
                "communities",
                "mek_generations_c1_ch",
            ),
            (
                VaultKey::SlotKeypair {
                    community: "c1".into(),
                },
                "communities",
                "slot_keypair_c1",
            ),
            (
                VaultKey::RegistryKeypair {
                    community: "c1".into(),
                },
                "communities",
                "registry_keypair_c1",
            ),
            (
                VaultKey::SlotSeed {
                    community: "c1".into(),
                },
                "communities",
                "slot_seed_c1",
            ),
            (VaultKey::AuditMacKey, "audit", "mac_key"),
            (VaultKey::AuditTail, "audit", "tail"),
        ];
        for (vk, namespace, key) in cases {
            assert_eq!(vk.namespace(), *namespace, "namespace mismatch for {vk:?}");
            assert_eq!(vk.key().as_ref(), *key, "key mismatch for {vk:?}");
        }
    }
}

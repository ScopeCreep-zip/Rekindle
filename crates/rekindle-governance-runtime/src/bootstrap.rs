//! Phase 18.e — bootstrap pipeline.
//!
//! Ported from `src-tauri/src/services/community/bootstrap.rs`.
//! Builds the `ControlPayload::BootstrapResponse` envelope that a joiner
//! receives during community join: snapshot governance entries + online
//! member list + wrapped owner keypair + wrapped per-channel MEKs +
//! re-encrypted recent messages.
//!
//! No DHT writes, no mesh broadcast — purely a read-side assembly task.

use std::collections::HashMap;

use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_secrets::mek::wrap_mek;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::member::MemberInfo;
use rekindle_types::mek::ChannelMekDelivery;
use rekindle_types::message::{BootstrapChannelMessages, BootstrapMessage};

use crate::deps::{GovernanceRuntimeDeps, MekSnapshot};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

const RECENT_MESSAGES_PER_CHANNEL: i64 = 50;

/// Architecture §13.4 — synthesize a typed `GovernanceEntry` snapshot
/// from the merged `GovernanceState` for a bootstrap response. The
/// joiner re-merges these entries; CRDT idempotence (Almeida 2016 §3)
/// makes a snapshot indistinguishable from the live DHT reads they'll
/// also perform.
fn snapshot_governance_entries(governance: &GovernanceState) -> Vec<GovernanceEntry> {
    let mut entries: Vec<GovernanceEntry> = Vec::new();

    if let Some(metadata) = &governance.metadata {
        entries.push(GovernanceEntry::CommunityMeta {
            name: Some(metadata.name.clone()),
            description: metadata.description.clone(),
            icon_hash: metadata.icon_hash.clone(),
            banner_hash: metadata.banner_hash.clone(),
            lamport: metadata.lamport,
        });
    }

    for (channel_id, channel) in &governance.channels {
        entries.push(GovernanceEntry::ChannelCreated {
            channel_id: *channel_id,
            name: channel.name.clone(),
            channel_type: channel.channel_type.clone(),
            record_key: channel.record_key.clone(),
            category_id: channel.category_id,
            position: channel.position,
            parent_voice_channel_id: channel.parent_voice_channel_id,
            lamport: channel.created_lamport,
        });
    }

    for (role_id, role) in &governance.roles {
        entries.push(GovernanceEntry::RoleDefinition {
            role_id: *role_id,
            name: role.name.clone(),
            permissions: role.permissions,
            position: role.position,
            color: role.color,
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: role.exclusion_group.clone(),
            lamport: role.lamport,
        });
    }

    for (member, role_ids) in &governance.role_assignments {
        for role_id in role_ids {
            entries.push(GovernanceEntry::RoleAssignment {
                target: member.clone(),
                role_id: *role_id,
                lamport: 0,
            });
        }
    }

    for banned in &governance.bans {
        entries.push(GovernanceEntry::BanEntry {
            target: banned.clone(),
            reason: None,
            lamport: 0,
        });
    }

    for (member, timeout) in &governance.timeouts {
        entries.push(GovernanceEntry::TimeoutEntry {
            target: member.clone(),
            duration_seconds: timeout.duration_seconds,
            reason: None,
            started_at: timeout.started_at,
            lamport: timeout.lamport,
        });
    }

    for (category_id, category) in &governance.categories {
        entries.push(GovernanceEntry::CategoryCreated {
            category_id: *category_id,
            name: category.name.clone(),
            position: category.position,
            lamport: category.created_lamport,
        });
    }

    if let Some(onboarding) = &governance.onboarding {
        entries.push(GovernanceEntry::OnboardingConfig {
            enabled: onboarding.enabled,
            mode: onboarding.mode.clone(),
            default_channels: onboarding.default_channels.clone(),
            questions: onboarding.questions.clone(),
            welcome_message: onboarding.welcome_message.clone(),
            guide_steps: onboarding.guide_steps.clone(),
            lamport: onboarding.lamport,
        });
    }

    if let Some(welcome_screen) = &governance.welcome_screen {
        entries.push(GovernanceEntry::WelcomeScreen {
            description: welcome_screen.description.clone(),
            channels: welcome_screen.channels.clone(),
            lamport: welcome_screen.lamport,
        });
    }

    for (invite_id, invite) in &governance.invites {
        entries.push(GovernanceEntry::InviteCreated {
            invite_id: *invite_id,
            code_hash: invite.code_hash.clone(),
            max_uses: invite.max_uses,
            expires_at: invite.expires_at,
            encrypted_secrets: invite.encrypted_secrets.clone(),
            lamport: invite.created_lamport,
        });
    }

    entries
}

fn wrap_key_material(
    sender_signing_key: &SigningKey,
    joiner_pseudonym: &[u8; 32],
    key_material: &[u8],
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    wrap_mek(sender_signing_key, joiner_pseudonym, key_material)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("wrap bootstrap key material: {e}")))
}

fn mek_to_wire(snapshot: &MekSnapshot) -> Vec<u8> {
    MediaEncryptionKey::from_bytes(snapshot.key_bytes, snapshot.generation).to_wire_bytes()
}

/// Build the encoded bytes of `ControlPayload::BootstrapResponse` for a
/// joiner. Returns the Cap'n Proto-encoded envelope ready for transport.
pub async fn build_bootstrap_response<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    governance_key: &str,
    joiner_pseudonym_hex: &str,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let joiner_pseudonym: [u8; 32] = hex::decode(joiner_pseudonym_hex)
        .map_err(|e| GovernanceRuntimeError::InvalidPseudonymHex(e.to_string()))?
        .try_into()
        .map_err(|_| {
            GovernanceRuntimeError::InvalidPseudonymHex(
                "joiner pseudonym must be 32 bytes".into(),
            )
        })?;

    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let bootstrap_signing_key = derive::derive_community_pseudonym(&identity_secret, governance_key);

    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;

    let governance_entries = snapshot_governance_entries(&gov_state);

    let online = deps.online_members(community_id);
    let member_list: Vec<MemberInfo> = online
        .into_iter()
        .map(|m| MemberInfo {
            pseudonym_key: m.pseudonym_hex,
            display_name: String::new(),
            role_ids: Vec::new(),
            status: m.status,
            timeout_until: None,
            route_blob: Some(m.route_blob),
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            last_seen: m.last_seen,
        })
        .collect();

    let wrapped_owner_keypair = match membership.dht_owner_keypair.as_ref() {
        Some(keypair_str) => wrap_key_material(
            &bootstrap_signing_key,
            &joiner_pseudonym,
            keypair_str.as_bytes(),
        )?,
        None => Vec::new(),
    };

    // Per-channel MEKs (architecture §5.2). Prefer per-channel cache;
    // fall back to community-wide MEK applied to every channel.
    let channel_meks: Vec<ChannelMekDelivery> = {
        let per_channel = deps.channel_meks_all(community_id);
        if per_channel.is_empty() {
            let community_mek = deps.community_mek(community_id);
            match community_mek {
                Some(mek) => {
                    let wrapped = wrap_key_material(
                        &bootstrap_signing_key,
                        &joiner_pseudonym,
                        &mek_to_wire(&mek),
                    )?;
                    membership
                        .channel_ids
                        .iter()
                        .map(|channel_id| ChannelMekDelivery {
                            channel_id: Some(channel_id.clone()),
                            generation: mek.generation,
                            wrapped_mek: wrapped.clone(),
                        })
                        .collect()
                }
                None => Vec::new(),
            }
        } else {
            per_channel
                .into_iter()
                .map(|ChannelMekSnapshotEntry { channel_id, mek }| {
                    let wrapped = wrap_key_material(
                        &bootstrap_signing_key,
                        &joiner_pseudonym,
                        &mek_to_wire(&mek),
                    )?;
                    Ok(ChannelMekDelivery {
                        channel_id: Some(channel_id),
                        generation: mek.generation,
                        wrapped_mek: wrapped,
                    })
                })
                .collect::<Result<Vec<_>, GovernanceRuntimeError>>()?
        }
    };

    // Architecture §14.4 — recent messages snapshot per channel, each
    // message re-encrypted under the historical MEK generation it was
    // stored with.
    let recent_messages = build_recent_messages(deps, community_id, &membership.channel_ids).await;

    let payload = ControlPayload::BootstrapResponse {
        governance_entries,
        member_list,
        channel_meks,
        recent_messages,
        wrapped_owner_keypair,
    };
    let envelope = CommunityEnvelope::Control(payload);
    let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&envelope)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("encode bootstrap response: {e}")))?;

    deps.emit_event(GovernanceRuntimeEvent::BootstrapResponseBuilt {
        community_id: community_id.to_string(),
        joiner_pseudonym_hex: joiner_pseudonym_hex.to_string(),
        bytes: bytes.len(),
    });

    Ok(bytes)
}

/// Local alias so the `channel_meks_all` Vec destructuring above reads cleanly.
type ChannelMekSnapshotEntry = crate::deps::ChannelMekSnapshot;

async fn build_recent_messages<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel_ids: &[String],
) -> Vec<BootstrapChannelMessages> {
    let mut out: Vec<BootstrapChannelMessages> = Vec::new();
    for channel_id in channel_ids {
        let rows = deps
            .recent_channel_messages(community_id, channel_id, RECENT_MESSAGES_PER_CHANNEL)
            .await;
        if rows.is_empty() {
            continue;
        }
        if let Some(group) = build_channel_envelope(deps, community_id, channel_id, &rows) {
            out.push(group);
        }
    }
    out
}

/// Re-encrypt each row under the MEK generation it was originally
/// stored with (architecture §5.2 line 1100). Historical generations
/// load from Stronghold via `load_historical_channel_mek` the first
/// time they're needed; the per-call HashMap caches them for the
/// remainder of the bundle build.
fn build_channel_envelope<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    rows: &[crate::deps::RecentMessageRow],
) -> Option<BootstrapChannelMessages> {
    let mut mek_by_gen: HashMap<u64, MediaEncryptionKey> = HashMap::new();
    let mut entries: Vec<BootstrapMessage> = Vec::with_capacity(rows.len());

    // Iterate oldest→newest so the joiner sees lamport-monotonic order
    // despite the underlying query returning DESC.
    for row in rows.iter().rev() {
        let mek = match mek_by_gen.entry(row.mek_generation) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let snapshot =
                    deps.load_historical_channel_mek(community_id, channel_id, row.mek_generation)?;
                v.insert(MediaEncryptionKey::from_bytes(
                    snapshot.key_bytes,
                    snapshot.generation,
                ))
            }
        };
        let Ok(ciphertext) = mek.encrypt(row.body.as_bytes()) else {
            continue;
        };
        entries.push(BootstrapMessage {
            message_id: row.message_id.clone(),
            sender_pseudonym: row.sender_pseudonym.clone(),
            ciphertext,
            mek_generation: row.mek_generation,
            timestamp: row.timestamp,
        });
    }

    if entries.is_empty() {
        return None;
    }
    Some(BootstrapChannelMessages {
        channel_id: channel_id.to_string(),
        messages: entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_governance::state::GovernanceState;
    use rekindle_types::id::ChannelId;

    #[test]
    fn snapshot_empty_governance_state_produces_empty_entries() {
        let gov = GovernanceState::default();
        assert!(snapshot_governance_entries(&gov).is_empty());
    }

    #[test]
    fn snapshot_includes_community_meta() {
        let gov = GovernanceState {
            metadata: Some(rekindle_governance::state::MetadataState {
                name: "Test".into(),
                description: Some("desc".into()),
                icon_hash: None,
                banner_hash: None,
                lamport: 1,
            }),
            ..Default::default()
        };
        let entries = snapshot_governance_entries(&gov);
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], GovernanceEntry::CommunityMeta { .. }));
    }

    #[test]
    fn snapshot_channels_become_channel_created_entries() {
        let channel_id = ChannelId([1u8; 16]);
        let mut channels = std::collections::HashMap::new();
        channels.insert(
            channel_id,
            rekindle_governance::state::ChannelState {
                name: "general".into(),
                channel_type: "text".into(),
                record_key: "rk".into(),
                category_id: None,
                position: 0,
                topic: None,
                forum_tags: None,
                slowmode_seconds: None,
                nsfw: None,
                parent_voice_channel_id: None,
                created_lamport: 5,
            },
        );
        let gov = GovernanceState {
            channels,
            ..Default::default()
        };
        let entries = snapshot_governance_entries(&gov);
        assert_eq!(entries.len(), 1);
        match &entries[0] {
            GovernanceEntry::ChannelCreated {
                channel_id: cid,
                name,
                lamport,
                ..
            } => {
                assert_eq!(cid, &channel_id);
                assert_eq!(name, "general");
                assert_eq!(*lamport, 5);
            }
            other => panic!("expected ChannelCreated, got {other:?}"),
        }
    }

    #[test]
    fn mek_to_wire_round_trips_through_from_wire_bytes() {
        let snap = MekSnapshot {
            generation: 42,
            key_bytes: [7u8; 32],
        };
        let wire = mek_to_wire(&snap);
        let mek = MediaEncryptionKey::from_wire_bytes(&wire).expect("wire bytes parse");
        assert_eq!(mek.generation(), 42);
        assert_eq!(*mek.as_bytes(), [7u8; 32]);
    }
}

use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_governance::state::GovernanceState;
use rekindle_protocol::dht::community::envelope::ControlPayload;
use rekindle_secrets::{derive, ed25519_dalek::SigningKey, mek::wrap_mek};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::member::MemberInfo;
use rekindle_types::mek::ChannelMekDelivery;
use rekindle_types::message::{BootstrapChannelMessages, BootstrapMessage};

use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::state::AppState;
use crate::state_helpers;

const RECENT_MESSAGES_PER_CHANNEL: i64 = 50;

struct RecentMessageRow {
    message_id: String,
    sender_pseudonym: String,
    body: String,
    timestamp: i64,
    /// Original `mek_generation` the message was written with. Architecture
    /// §5.2 line 1100 puts the generation in every ciphertext envelope so
    /// the receiver can pick the right key; we preserve that here so a
    /// joiner who later receives the actual SMPL record entries sees a
    /// consistent generation across both delivery paths.
    mek_generation: u64,
}

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
                // No per-assignment lamport in merged state — use 0; readers
                // tie-break by author pseudonym ordering.
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
) -> Result<Vec<u8>, String> {
    wrap_mek(sender_signing_key, joiner_pseudonym, key_material)
        .map_err(|e| format!("wrap bootstrap key material: {e}"))
}

pub async fn build_bootstrap_response(
    state: &Arc<AppState>,
    community_id: &str,
    governance_key: &str,
    joiner_pseudonym_hex: &str,
) -> Result<Vec<u8>, String> {
    let joiner_pseudonym: [u8; 32] = hex::decode(joiner_pseudonym_hex)
        .map_err(|e| format!("invalid joiner pseudonym hex: {e}"))?
        .try_into()
        .map_err(|_| "joiner pseudonym must be 32 bytes")?;

    let identity_secret = state
        .identity_secret
        .lock()
        .as_ref()
        .copied()
        .ok_or("identity secret not available")?;
    let bootstrap_signing_key =
        derive::derive_community_pseudonym(&identity_secret, governance_key);

    let (governance_entries, member_list, wrapped_owner_keypair) = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or("community not found for bootstrap")?;
        let governance_state = community
            .governance_state
            .as_ref()
            .ok_or("governance state not cached")?;
        let governance_entries = snapshot_governance_entries(governance_state);
        let member_list: Vec<MemberInfo> = community
            .gossip
            .as_ref()
            .map(|gossip| {
                gossip
                    .online_members
                    .iter()
                    .map(|(pseudonym_key, member)| MemberInfo {
                        pseudonym_key: pseudonym_key.clone(),
                        display_name: String::new(),
                        role_ids: Vec::new(),
                        status: member.status.clone(),
                        timeout_until: None,
                        route_blob: Some(member.route_blob.clone()),
                        bio: None,
                        pronouns: None,
                        theme_color: None,
                        badges: Vec::new(),
                        last_seen: member.last_seen,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let wrapped_owner_keypair = community
            .dht_owner_keypair
            .as_ref()
            .map(|owner_keypair| {
                wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    owner_keypair.as_bytes(),
                )
            })
            .transpose()?
            .unwrap_or_default();

        (governance_entries, member_list, wrapped_owner_keypair)
    };

    let channel_meks: Vec<ChannelMekDelivery> = {
        let channels = {
            let communities = state.communities.read();
            communities
                .get(community_id)
                .map(|community| {
                    community
                        .channels
                        .iter()
                        .map(|channel| channel.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let channel_mek_cache = state.channel_mek_cache.lock();
        let mut entries: Vec<ChannelMekDelivery> = channel_mek_cache
            .iter()
            .filter(|((cid, _), _)| cid == community_id)
            .map(|((_, channel_id), mek)| {
                let wrapped = wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    &mek.to_wire_bytes(),
                )?;
                Ok(ChannelMekDelivery {
                    channel_id: Some(channel_id.clone()),
                    generation: mek.generation(),
                    wrapped_mek: wrapped,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if entries.is_empty() {
            let community_mek_cache = state.mek_cache.lock();
            if let Some(mek) = community_mek_cache.get(community_id) {
                let wrapped = wrap_key_material(
                    &bootstrap_signing_key,
                    &joiner_pseudonym,
                    &mek.to_wire_bytes(),
                )?;
                for channel_id in channels {
                    entries.push(ChannelMekDelivery {
                        channel_id: Some(channel_id),
                        generation: mek.generation(),
                        wrapped_mek: wrapped.clone(),
                    });
                }
            }
        }
        entries
    };

    // Architecture §14.4: bootstrap bundles also carry the last N
    // messages per channel so the joiner has scrollback immediately,
    // skipping the slower history-catchup ad path. Re-encrypt under the
    // current MEK so the joiner can decrypt with the wrapped MEKs in
    // `channel_meks` (single key for the snapshot).
    let recent_messages = build_recent_messages(state, community_id).await;

    let payload = ControlPayload::BootstrapResponse {
        governance_entries,
        member_list,
        channel_meks,
        recent_messages,
        wrapped_owner_keypair,
    };
    let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(payload);
    rekindle_protocol::capnp_envelope::encode_community_envelope(&envelope)
        .map_err(|e| format!("encode bootstrap response: {e}"))
}

async fn build_recent_messages(
    state: &Arc<AppState>,
    community_id: &str,
) -> Vec<BootstrapChannelMessages> {
    let Some(pool) = db_pool_from_state(state) else {
        return Vec::new();
    };
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return Vec::new();
    };
    let channel_ids: Vec<String> = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .map(|community| {
                community
                    .channels
                    .iter()
                    .map(|channel| channel.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out: Vec<BootstrapChannelMessages> = Vec::new();
    for channel_id in channel_ids {
        let rows = load_recent_channel_messages(&pool, &owner_key, community_id, &channel_id).await;
        if rows.is_empty() {
            continue;
        }
        if let Some(group) = build_channel_envelope(state, community_id, &channel_id, &rows) {
            out.push(group);
        }
    }
    out
}

fn db_pool_from_state(state: &Arc<AppState>) -> Option<DbPool> {
    use tauri::Manager as _;
    let app_handle = state_helpers::app_handle(state)?;
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    Some(pool.inner().clone())
}

async fn load_recent_channel_messages(
    pool: &DbPool,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> Vec<RecentMessageRow> {
    let owner = owner_key.to_string();
    let cid = community_id.to_string();
    let chan = channel_id.to_string();
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT message_id, sender_key, body, timestamp, mek_generation \
             FROM messages \
             WHERE owner_key = ?1 AND community_id = ?2 \
               AND conversation_type = 'channel' AND conversation_id = ?3 \
             ORDER BY timestamp DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![owner, cid, chan, RECENT_MESSAGES_PER_CHANNEL],
            |row| {
                Ok(RecentMessageRow {
                    message_id: row
                        .get::<_, Option<String>>(0)?
                        .unwrap_or_default(),
                    sender_pseudonym: row.get::<_, String>(1)?,
                    body: row.get::<_, String>(2)?,
                    timestamp: row.get::<_, i64>(3)?,
                    mek_generation: row
                        .get::<_, Option<i64>>(4)?
                        .unwrap_or(0)
                        .max(0)
                        .cast_unsigned(),
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await
}

/// Re-encrypt each message under the MEK generation it was originally
/// stored with (architecture §5.2 line 1100). Historical MEKs are
/// materialized from Stronghold via `keystore::load_channel_mek_generation`
/// the first time they're needed; the result lives in
/// `channel_mek_cache` for the rest of the bundle build.
fn build_channel_envelope(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    rows: &[RecentMessageRow],
) -> Option<BootstrapChannelMessages> {
    use std::collections::HashMap;
    let mut mek_by_gen: HashMap<u64, MediaEncryptionKey> = HashMap::new();
    let mut entries: Vec<BootstrapMessage> = Vec::with_capacity(rows.len());
    // Iterate oldest→newest so the joiner sees lamport-monotonic order
    // despite the SELECT pulling DESC.
    for row in rows.iter().rev() {
        let mek = match mek_by_gen.entry(row.mek_generation) {
            std::collections::hash_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let Some(mek) =
                    load_historical_channel_mek(state, community_id, channel_id, row.mek_generation)
                else {
                    continue;
                };
                v.insert(mek)
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

/// Look up the MEK for a specific channel generation. Tries the
/// in-memory cache first, then falls back to Stronghold's per-generation
/// vault entry. Returns `None` if neither source has the key, in which
/// case the bootstrap simply skips messages from that generation rather
/// than ship undecryptable data.
fn load_historical_channel_mek(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    generation: u64,
) -> Option<MediaEncryptionKey> {
    {
        let cache = state.channel_mek_cache.lock();
        if let Some(mek) = cache.get(&(community_id.to_string(), channel_id.to_string())) {
            if mek.generation() == generation {
                return Some(MediaEncryptionKey::from_bytes(*mek.as_bytes(), generation));
            }
        }
    }
    let app_handle = state_helpers::app_handle(state)?;
    use tauri::Manager as _;
    let keystore: tauri::State<'_, crate::keystore::KeystoreHandle> = app_handle.state();
    let guard = keystore.lock();
    let ks = guard.as_ref()?;
    crate::keystore::load_channel_mek_generation(ks, community_id, channel_id, generation)
}

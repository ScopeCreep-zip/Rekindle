use std::sync::Arc;

use rekindle_protocol::dht::DHTManager;
use tauri::Manager;

use crate::state::AppState;
use crate::state_helpers;

use super::current_presence_status;

fn presence_event_id(event_id: &str) -> rekindle_types::id::EventId {
    let hash = blake3::hash(event_id.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    rekindle_types::id::EventId(bytes)
}

pub(super) async fn ensure_registry_open(
    state: &Arc<AppState>,
    community_id: &str,
    mgr: &DHTManager,
    registry_key: &str,
) -> Result<(), String> {
    let records_open = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .is_some_and(|c| c.open_community_records.records_open)
    };
    if records_open {
        return Ok(());
    }

    let (registry_kp, slot_kp) = {
        let communities = state.communities.read();
        let c = communities.get(community_id);
        (
            c.and_then(|c| c.registry_owner_keypair.clone()),
            c.and_then(|c| c.slot_keypair.clone()),
        )
    };
    let writer_kp = registry_kp.or(slot_kp);
    let opened = if let Some(ref kp_str) = writer_kp {
        if let Ok(kp) = kp_str.parse::<veilid_core::KeyPair>() {
            mgr.open_record_writable(registry_key, kp).await.is_ok()
        } else {
            false
        }
    } else {
        false
    };
    if !opened {
        mgr.open_record(registry_key)
            .await
            .map_err(|e| format!("presence_poll: failed to open registry: {e}"))?;
    }
    let registry_key_owned = registry_key.to_string();
    state_helpers::track_open_records(state, std::slice::from_ref(&registry_key_owned));
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.open_community_records.registry_key = Some(registry_key.to_string());
            cs.open_community_records.registry_writer = writer_kp;
            cs.open_community_records.records_open = true;
        }
    }
    tracing::debug!(community = %community_id, "presence_poll: re-opened registry after restart");
    Ok(())
}

/// Snapshot of our own per-community profile fields plus event RSVPs.
/// Read once under the `communities` read lock so the presence write
/// path doesn't have to juggle six clones inline.
struct SelfPresenceSnapshot {
    event_rsvps: Vec<rekindle_types::presence::EventRSVP>,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<u32>,
    badges: Vec<String>,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
}

fn read_self_presence_snapshot(
    state: &Arc<AppState>,
    community_id: &str,
) -> SelfPresenceSnapshot {
    let communities = state.communities.read();
    let Some(community) = communities.get(community_id) else {
        return SelfPresenceSnapshot {
            event_rsvps: Vec::new(),
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            avatar_ref: None,
            banner_ref: None,
        };
    };
    let event_rsvps = community
        .my_event_rsvps
        .iter()
        .map(|(event_id, status)| rekindle_types::presence::EventRSVP {
            event_id: presence_event_id(event_id),
            status: status.clone(),
        })
        .collect::<Vec<_>>();
    SelfPresenceSnapshot {
        event_rsvps,
        bio: community.my_bio.clone(),
        pronouns: community.my_pronouns.clone(),
        theme_color: community.my_theme_color,
        badges: community.my_badges.clone(),
        avatar_ref: community.my_avatar_ref.clone(),
        banner_ref: community.my_banner_ref.clone(),
    }
}

pub(super) async fn write_our_presence(
    state: &Arc<AppState>,
    community_id: &str,
    rc: &veilid_core::RoutingContext,
    registry_key: &str,
    my_pseudonym: &str,
    my_subkey_index: Option<u32>,
    slot_keypair_str: Option<&String>,
    has_slot_seed: bool,
    history_ranges: Vec<rekindle_types::presence::HistoryRange>,
) {
    if let (Some(subkey_idx), Some(kp_str)) = (my_subkey_index, slot_keypair_str) {
        let our_route_blob = state_helpers::our_route_blob(state);
        if our_route_blob.is_none() {
            tracing::warn!(
                community = %community_id,
                "presence_poll_tick: our_route_blob is None — peers cannot reach us"
            );
        }
        let snapshot = read_self_presence_snapshot(state, community_id);
        // W11.2 — encrypt history_ranges under current community MEK
        // before writing the registry row. `None` is the correct
        // graceful-skip when (a) we have no ranges to advertise yet,
        // or (b) MEK isn't cached (the receive path already falls back
        // to direct DHT reads in that case).
        let history_ranges_encrypted = if history_ranges.is_empty() {
            None
        } else {
            encrypt_history_ranges(state, community_id, &history_ranges)
        };
        let mut presence = rekindle_types::presence::MemberPresence {
            pseudonym_key: rekindle_types::id::PseudonymKey(
                hex::decode(my_pseudonym)
                    .ok()
                    .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                    .unwrap_or([0u8; 32]),
            ),
            display_name: Some(state_helpers::identity_display_name(state)),
            status: current_presence_status(state).into(),
            route_blob: our_route_blob.unwrap_or_default(),
            last_heartbeat: rekindle_utils::timestamp_secs(),
            event_rsvps: snapshot.event_rsvps,
            history_ranges_encrypted,
            bio: snapshot.bio,
            pronouns: snapshot.pronouns,
            theme_color: snapshot.theme_color,
            badges: snapshot.badges,
            avatar_ref: snapshot.avatar_ref,
            banner_ref: snapshot.banner_ref,
            ..Default::default()
        };
        // Architecture §26 W26 — sign before publishing so receivers can
        // verify the presence row actually came from `pseudonym_key`.
        match state_helpers::pseudonym_credentials(state, community_id) {
            Ok((_, signing_key)) => {
                let sig = rekindle_secrets::derive::sign_with_pseudonym(
                    &signing_key,
                    &presence.signing_bytes(),
                );
                presence.signature = sig.to_vec();
            }
            Err(e) => {
                tracing::warn!(
                    community = %community_id,
                    error = %e,
                    "skipping presence write: pseudonym credentials unavailable",
                );
                return;
            }
        }
        if let (Ok(writer_kp), Ok(reg_key)) = (
            kp_str.parse::<veilid_core::KeyPair>(),
            registry_key.parse::<veilid_core::RecordKey>(),
        ) {
            let write_opts = veilid_core::SetDHTValueOptions {
                writer: Some(writer_kp),
                ..Default::default()
            };
            if let Err(e) = rc
                .set_dht_value(
                    reg_key,
                    subkey_idx,
                    serde_json::to_vec(&presence).unwrap_or_default(),
                    Some(write_opts),
                )
                .await
            {
                tracing::debug!(
                    community = %community_id,
                    subkey = subkey_idx,
                    error = %e,
                    "failed to write presence to registry"
                );
            }
        }
    } else {
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed,
            "cannot write presence — missing slot keypair or subkey index"
        );
    }
}

/// `(segment_index, local_subkey, presence)` — the registry scan tuple.
/// Plate Gate (architecture §15) extends discovery from a single registry
/// to N segment registries; downstream tracks per-row segment context for
/// the SQLite write so a member's location is fully recoverable.
pub(crate) type DiscoveredRow = (u32, u32, rekindle_types::presence::MemberPresence);

/// Materialized DB row built from a discovered `MemberPresence`. Lifted
/// out of the persist function so the SQL parameter list maps cleanly
/// to named fields and avoids the `clippy::type_complexity` smell.
struct MemberPersistRow {
    pseudonym_key: String,
    display_name: Option<String>,
    role_ids_json: String,
    subkey_index: i64,
    segment_index: i64,
    bio: Option<String>,
    pronouns: Option<String>,
    theme_color: Option<i64>,
    badges_json: String,
    avatar_ref: Option<String>,
    banner_ref: Option<String>,
}

impl MemberPersistRow {
    fn from_presence(
        presence: &rekindle_types::presence::MemberPresence,
        segment_index: u32,
        subkey: u32,
        member_roles: &std::collections::HashMap<String, Vec<u32>>,
    ) -> Self {
        let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
        let role_ids_json = serde_json::to_string(
            member_roles
                .get(&pseudonym_hex)
                .cloned()
                .unwrap_or_else(|| vec![0])
                .as_slice(),
        )
        .unwrap_or_else(|_| "[0]".to_string());
        let badges_json =
            serde_json::to_string(&presence.badges).unwrap_or_else(|_| "[]".into());
        Self {
            pseudonym_key: pseudonym_hex,
            display_name: presence.display_name.clone(),
            role_ids_json,
            subkey_index: i64::from(subkey),
            segment_index: i64::from(segment_index),
            bio: presence.bio.clone(),
            pronouns: presence.pronouns.clone(),
            theme_color: presence.theme_color.map(i64::from),
            badges_json,
            avatar_ref: presence.avatar_ref.clone(),
            banner_ref: presence.banner_ref.clone(),
        }
    }
}

pub(crate) fn persist_discovered_registry_members(
    state: &Arc<AppState>,
    community_id: &str,
    discovered_members: &[DiscoveredRow],
    member_roles: &std::collections::HashMap<String, Vec<u32>>,
    banned_members: &std::collections::HashSet<String>,
) {
    use tauri::Emitter as _;

    let app_handle = { state.app_handle.read().clone() };
    let Some(app_handle) = app_handle else { return };
    let pool: tauri::State<'_, crate::db::DbPool> = app_handle.state();
    let Ok(owner_key) = state_helpers::current_owner_key(state) else {
        return;
    };
    let cid = community_id.to_string();
    let rows: Vec<MemberPersistRow> = discovered_members
        .iter()
        .map(|(segment_index, subkey, presence)| {
            MemberPersistRow::from_presence(presence, *segment_index, *subkey, member_roles)
        })
        .collect();

    // Architecture §15 — emit `MemberDiscovered` for rows the local
    // client hasn't seen before. Detection is done against the
    // in-memory `known_members` set; freshly-discovered pseudonyms
    // are added to that set so subsequent polls don't re-emit. The
    // emission happens before SQLite persistence so the UI can react
    // immediately; persistence runs on the DB worker thread below.
    let newly_discovered: Vec<&MemberPersistRow> = {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            rows.iter()
                .filter(|row| cs.known_members.insert(row.pseudonym_key.clone()))
                .collect()
        } else {
            Vec::new()
        }
    };
    for row in newly_discovered {
        let _ = app_handle.emit(
            "community-event",
            crate::channels::CommunityEvent::MemberDiscovered {
                community_id: community_id.to_string(),
                pseudonym_key: row.pseudonym_key.clone(),
                display_name: row.display_name.clone().unwrap_or_default(),
                subkey_index: u32::try_from(row.subkey_index).unwrap_or(0),
            },
        );
    }

    let banned_rows: Vec<String> = banned_members.iter().cloned().collect();
    let joined_at = crate::db::timestamp_now();
    crate::db_helpers::db_fire(
        pool.inner(),
        "persist discovered registry members",
        move |conn| {
            for banned in &banned_rows {
                conn.execute(
                    "DELETE FROM community_members WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym_key = ?3",
                    rusqlite::params![owner_key, cid, banned],
                )?;
            }
            for row in &rows {
                conn.execute(
                    "INSERT INTO community_members \
                 (owner_key, community_id, pseudonym_key, display_name, role_ids, joined_at, \
                  subkey_index, segment_index, bio, pronouns, theme_color, badges, \
                  avatar_ref, banner_ref) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                 ON CONFLICT(owner_key, community_id, pseudonym_key) DO UPDATE SET \
                   display_name = excluded.display_name, \
                   role_ids = excluded.role_ids, \
                   subkey_index = excluded.subkey_index, \
                   segment_index = excluded.segment_index, \
                   bio = excluded.bio, \
                   pronouns = excluded.pronouns, \
                   theme_color = excluded.theme_color, \
                   badges = excluded.badges, \
                   avatar_ref = excluded.avatar_ref, \
                   banner_ref = excluded.banner_ref",
                    rusqlite::params![
                        owner_key,
                        cid,
                        row.pseudonym_key,
                        row.display_name,
                        row.role_ids_json,
                        joined_at,
                        row.subkey_index,
                        row.segment_index,
                        row.bio,
                        row.pronouns,
                        row.theme_color,
                        row.badges_json,
                        row.avatar_ref,
                        row.banner_ref,
                    ],
                )?;
            }
            Ok(())
        },
    );
}

/// W11.2 — encrypt the local history ranges with the current community
/// MEK. Returns `None` when no MEK is cached (community freshly joined,
/// pre-MEK-distribution); the caller treats that as "skip this field"
/// and still writes the rest of the presence row.
fn encrypt_history_ranges(
    state: &Arc<AppState>,
    community_id: &str,
    ranges: &[rekindle_types::presence::HistoryRange],
) -> Option<rekindle_types::presence::EncryptedHistoryRanges> {
    let mek = {
        let cache = state.mek_cache.lock();
        cache.get(community_id).cloned()?
    };
    let plaintext = serde_json::to_vec(ranges).ok()?;
    let ciphertext = mek.encrypt(&plaintext).ok()?;
    Some(rekindle_types::presence::EncryptedHistoryRanges {
        mek_generation: mek.generation(),
        ciphertext,
    })
}

/// W11.2 — decrypt history ranges from a peer's presence row using the
/// MEK generation declared by the sender. Returns `None` when:
/// - we don't have that MEK generation cached (we joined after rotation
///   or haven't received the wrap yet);
/// - decryption fails (corrupted ciphertext, wrong key, etc.).
///
/// The caller treats `None` as "skip this peer's history advertisement
/// for now" — they fall through to direct DHT reads for catchup, which
/// is the existing behavior pre-encryption.
pub(in crate::services::community) fn decrypt_history_ranges(
    state: &Arc<AppState>,
    community_id: &str,
    encrypted: &rekindle_types::presence::EncryptedHistoryRanges,
) -> Option<Vec<rekindle_types::presence::HistoryRange>> {
    let mek = {
        let cache = state.mek_cache.lock();
        cache.get(community_id).cloned()?
    };
    if mek.generation() != encrypted.mek_generation {
        // Mismatched generation — could be a peer publishing under an
        // older MEK we've already rotated away from, or a fresher MEK
        // we haven't received yet. Either way, we can't decrypt.
        return None;
    }
    let plaintext = mek.decrypt(&encrypted.ciphertext).ok()?;
    serde_json::from_slice(&plaintext).ok()
}

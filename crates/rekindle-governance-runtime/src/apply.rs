//! Phase 18.c — apply pipeline.
//!
//! Ported from `src-tauri/src/services/community/governance.rs::write_entry`.
//! Pure pipeline: read existing entries → permission validate → DHT
//! write with M9.5 conflict/verify → mesh broadcast → local CRDT merge
//! → UI snapshot emit. All side effects flow through the
//! `GovernanceRuntimeDeps` trait.

use rekindle_governance::{compact, merge, validate};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_secrets::derive;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::overflow;

/// Warn threshold (~85% of the ~4112 B SMPL per-subkey cap) for the **primary**
/// governance subkey. With `GovernanceOverflow` spill (see `overflow`), the
/// primary page is bounded under [`overflow::PRIMARY_PAGE_BUDGET`] before we get
/// here, so this should rarely fire — a primary near the cap signals the paging
/// budget drifted, not impending data loss.
const SMPL_SUBKEY_WARN_BYTES: usize = 3500;

/// Hard SMPL per-subkey cap for a 255-slot governance record:
/// `MAX_RECORD_DATA_SIZE (1_048_576) / subkey_count (255)`. A `set_dht_value`
/// over this fails inside Veilid as a generic "failed schema validation" with no
/// size detail. With overflow paging the primary page is partitioned under this
/// cap, so this guard is now a **safety net** (a single entry larger than a
/// whole page) rather than the accumulation wall it once was — kept so any
/// regression names itself with a typed error + per-kind breakdown instead of
/// Veilid's opaque string.
const SMPL_SUBKEY_MAX_BYTES: usize = 1_048_576 / 255;

/// Architecture §6 — classify a governance entry by which UI snapshot
/// it invalidates so we can emit the right event after a successful
/// CRDT apply. `Roles` triggers a `RolesChanged` snapshot;
/// `ChannelsOrCategories` triggers `ChannelsUpdated`. Returning both
/// false is intentional for entries that touch nothing the UI caches
/// (e.g. lifecycle events handled elsewhere).
#[derive(Default, Clone, Copy)]
struct EntryAffects {
    roles: bool,
    channels_or_categories: bool,
}

fn classify_entry(entry: &GovernanceEntry) -> EntryAffects {
    match entry {
        // Role definition + archive change the role list itself;
        // assignment/unassignment change a member's role_ids and are
        // handled by `MemberRolesChanged` elsewhere.
        GovernanceEntry::RoleDefinition { .. } | GovernanceEntry::RoleArchived { .. } => {
            EntryAffects {
                roles: true,
                ..EntryAffects::default()
            }
        }
        GovernanceEntry::ChannelCreated { .. }
        | GovernanceEntry::ChannelArchived { .. }
        | GovernanceEntry::ChannelUpdated { .. }
        | GovernanceEntry::CategoryCreated { .. }
        | GovernanceEntry::CategoryArchived { .. }
        | GovernanceEntry::CategoryUpdated { .. } => EntryAffects {
            channels_or_categories: true,
            ..EntryAffects::default()
        },
        _ => EntryAffects::default(),
    }
}

/// Per-kind entry counts for overflow diagnostics, e.g.
/// `"ChannelCreated=2, InviteCreated=8, RoleDefinition=3"`. Only runs on the
/// should-never-happen `SubkeyOverflow` path.
fn entry_kind_breakdown(entries: &[GovernanceEntry]) -> String {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for e in entries {
        *counts.entry(entry_kind(e)).or_insert(0) += 1;
    }
    counts
        .iter()
        .map(|(kind, n)| format!("{kind}={n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn entry_kind(entry: &GovernanceEntry) -> &'static str {
    match entry {
        GovernanceEntry::InviteCreated { .. } => "InviteCreated",
        GovernanceEntry::InviteRevoked { .. } => "InviteRevoked",
        GovernanceEntry::ChannelCreated { .. } => "ChannelCreated",
        GovernanceEntry::ChannelUpdated { .. } => "ChannelUpdated",
        GovernanceEntry::ChannelArchived { .. } => "ChannelArchived",
        GovernanceEntry::RoleDefinition { .. } => "RoleDefinition",
        GovernanceEntry::RoleAssignment { .. } => "RoleAssignment",
        GovernanceEntry::MEKGenerationBump { .. } => "MEKGenerationBump",
        GovernanceEntry::CommunityMeta { .. } => "CommunityMeta",
        _ => "other",
    }
}

/// Apply a governance entry: sign + write to our SMPL subkey, gossip
/// `GovernanceUpdated`, merge locally, emit UI snapshot event.
///
/// Returns `Ok(())` on success. On M9.5 write conflict or read-back
/// mismatch, the local merge + UI emit is skipped (the entry never
/// landed on the network, so the caller's optimistic UI must roll back).
pub async fn write_entry<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), GovernanceRuntimeError> {
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;

    let my_pseudo_hex = membership
        .my_pseudonym_hex
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_string()))?;
    let pseudo_bytes: [u8; 32] = hex::decode(&my_pseudo_hex)
        .map_err(|e| GovernanceRuntimeError::InvalidPseudonymHex(e.to_string()))?
        .try_into()
        .map_err(|_| {
            GovernanceRuntimeError::InvalidPseudonymHex("pseudonym must be 32 bytes".into())
        })?;
    let pseudo = PseudonymKey(pseudo_bytes);

    if !validate::validate_write(&pseudo, &entry, &gov_state) {
        return Err(GovernanceRuntimeError::PermissionDenied);
    }

    let gov_key_str = membership
        .governance_key
        .ok_or_else(|| GovernanceRuntimeError::GovernanceKeyMissing(community_id.to_string()))?;
    let my_slot = membership
        .my_subkey_index
        .ok_or_else(|| GovernanceRuntimeError::SlotIndexMissing(community_id.to_string()))?;
    let slot_kp_str = membership
        .slot_keypair
        .ok_or_else(|| GovernanceRuntimeError::SlotKeypairMissing(community_id.to_string()))?;

    // Architecture §"Follow GovernanceOverflow pointers" (line 1609) — reassemble
    // this author's FULL logical entry set across the primary SMPL subkey + every
    // overflow record in our chain, re-verifying each payload's signature (W26)
    // against our own pseudonym before accumulating. Capturing the existing
    // overflow record keys lets us reuse (open) them rather than re-create.
    let chain = overflow::read_my_chain(deps, &gov_key_str, my_slot, &pseudo).await?;
    let mut my_entries = chain.entries;
    my_entries.push(entry.clone());

    // Architecture §4.2 Strategy 1 — compact the WHOLE logical set before
    // re-paging (the genesis guard + create/archive lifecycle pairing require the
    // full set). Dead lifecycles and superseded single-key metadata are dropped.
    let my_entries = compact::compact_author_entries(my_entries, rekindle_utils::timestamp_secs());

    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let pseudonym_signing_key = derive::derive_community_pseudonym(&identity_secret, community_id);

    // Partition the compacted log into pages: page 0 is the primary SMPL subkey
    // (small budget), pages ≥1 are member-owned overflow DFLT(1) records (~8×
    // larger). An entry larger than a whole overflow page is unsplittable → the
    // should-never-happen SubkeyOverflow safety net (real entries are < ~500 B).
    let pages = match overflow::partition_into_pages(
        my_entries,
        overflow::PRIMARY_PAGE_BUDGET,
        overflow::OVERFLOW_PAGE_BUDGET,
    ) {
        Ok(pages) => pages,
        Err(bytes) => {
            tracing::error!(
                entry_bytes = bytes,
                cap = overflow::OVERFLOW_PAGE_BUDGET,
                "a single governance entry exceeds the overflow page budget; unsplittable"
            );
            return Err(GovernanceRuntimeError::SubkeyOverflow {
                bytes,
                cap: overflow::OVERFLOW_PAGE_BUDGET,
            });
        }
    };

    // Write overflow pages highest-index-first so each parent points at an
    // already-published child. Overflow owner keypairs are DERIVED (not stored);
    // reuse a record key already in our chain, else create it once.
    let mut next_key: Option<String> = None;
    let mut written_overflow_keys: Vec<String> = Vec::new();
    for i in (1..pages.len()).rev() {
        let page_index = u32::try_from(i).expect("overflow page index fits u32");
        let owner_writer =
            overflow::overflow_owner_writer(deps, &identity_secret, community_id, page_index);
        let rec_key = match chain.overflow_keys.get(i - 1) {
            Some(existing) => existing.clone(),
            None => deps.create_overflow_record(owner_writer.clone()).await?,
        };
        overflow::write_overflow_page(
            deps,
            &rec_key,
            &pages[i],
            next_key.clone(),
            &pseudonym_signing_key,
            &pseudo,
            owner_writer,
        )
        .await?;
        written_overflow_keys.push(rec_key.clone());
        next_key = Some(rec_key);
    }

    // Register our own spill pages in the community's record inventory so they
    // are warmed (§14.1), opened+tracked (§10), rehydrated on restart (D5), and
    // closed on leave (§10) like every other community record — the author MUST
    // keep its overflow records alive or new channels vanish from joiners.
    if !written_overflow_keys.is_empty() {
        deps.register_governance_overflow_keys(community_id, &written_overflow_keys);
    }

    // Build + sign the primary subkey (page 0), pointing at the first overflow
    // record (or `None` when everything fit one page). The pointer is bound into
    // the signature (`signing_bytes`, the canonical `rekindle-gov-subkey-v1`
    // domain).
    let (primary_payload, payload) =
        overflow::build_signed_payload(&pages[0], next_key, &pseudonym_signing_key, &pseudo)?;

    // The primary SMPL subkey is bounded by Veilid's per-subkey cap
    // (`min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE / subkey_count)` ≈ 4112 B for a
    // 255-slot record). Paging keeps page 0 under PRIMARY_PAGE_BUDGET, so these
    // guards are now a safety net rather than an expected path.
    if payload.len() >= SMPL_SUBKEY_WARN_BYTES {
        tracing::warn!(
            payload_bytes = payload.len(),
            entry_count = primary_payload.entries.len(),
            subkey = my_slot,
            "governance primary subkey payload approaching the SMPL per-subkey cap (~4112 B)"
        );
    } else {
        tracing::debug!(
            payload_bytes = payload.len(),
            entry_count = primary_payload.entries.len(),
            subkey = my_slot,
            "writing governance primary subkey payload"
        );
    }
    if payload.len() > SMPL_SUBKEY_MAX_BYTES {
        tracing::error!(
            payload_bytes = payload.len(),
            cap = SMPL_SUBKEY_MAX_BYTES,
            subkey = my_slot,
            breakdown = %entry_kind_breakdown(&primary_payload.entries),
            "governance primary subkey exceeds the SMPL per-subkey cap even after paging"
        );
        return Err(GovernanceRuntimeError::SubkeyOverflow {
            bytes: payload.len(),
            cap: SMPL_SUBKEY_MAX_BYTES,
        });
    }

    // M9.5 — set_dht_value returns Some(stale) when our write was NOT
    // accepted by the network. Surface as WriteConflict so the caller
    // doesn't emit GovernanceUpdated for a write that didn't land.
    let write_outcome = deps
        .set_dht_value(&gov_key_str, my_slot, payload.clone(), Some(slot_kp_str))
        .await?;
    if let Some(stale) = write_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    // M9.5 — read-back verification. Even on local-set success, network
    // propagation can fail. Force a fresh read and confirm the payload
    // is what the network now serves.
    let verify = deps
        .get_dht_value(&gov_key_str, my_slot, true)
        .await?
        .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
    if verify != payload {
        return Err(GovernanceRuntimeError::VerifyMismatch {
            read: verify.len(),
            written: payload.len(),
        });
    }

    let notification = CommunityEnvelope::Control(ControlPayload::GovernanceUpdated {
        governance_key: gov_key_str,
        subkey_index: my_slot,
        lamport_ts: entry.lamport(),
    });
    deps.send_to_mesh(community_id, &notification)?;

    // Local CRDT merge after the network confirms the entry landed.
    if let Some(mut current_state) = deps.governance_state(community_id) {
        merge::apply_entry(&pseudo, &entry, &mut current_state);
        deps.set_governance_state(community_id, current_state);
    }

    deps.emit_event(GovernanceRuntimeEvent::GovernanceEntryApplied {
        community_id: community_id.to_string(),
        entry: Box::new(entry.clone()),
    });

    let affects = classify_entry(&entry);
    if affects.roles {
        deps.emit_event(GovernanceRuntimeEvent::RolesChanged {
            community_id: community_id.to_string(),
        });
    }
    if affects.channels_or_categories {
        deps.emit_event(GovernanceRuntimeEvent::ChannelsUpdated {
            community_id: community_id.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::id::ChannelId;

    #[test]
    fn classify_role_definition_marks_roles() {
        let entry = GovernanceEntry::RoleDefinition {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            name: "test".into(),
            permissions: 0,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_role_archived_marks_roles() {
        let entry = GovernanceEntry::RoleArchived {
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(affects.roles);
        assert!(!affects.channels_or_categories);
    }

    #[test]
    fn classify_channel_created_marks_channels() {
        let entry = GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([0u8; 16]),
            name: "general".into(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn classify_category_updated_marks_channels() {
        let entry = GovernanceEntry::CategoryUpdated {
            category_id: rekindle_types::id::CategoryId([0u8; 16]),
            name: Some("renamed".into()),
            position: None,
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(affects.channels_or_categories);
    }

    #[test]
    fn classify_role_assignment_affects_nothing() {
        let entry = GovernanceEntry::RoleAssignment {
            target: PseudonymKey([0u8; 32]),
            role_id: rekindle_types::id::RoleId([0u8; 16]),
            lamport: 1,
        };
        let affects = classify_entry(&entry);
        assert!(!affects.roles);
        assert!(!affects.channels_or_categories);
    }
}

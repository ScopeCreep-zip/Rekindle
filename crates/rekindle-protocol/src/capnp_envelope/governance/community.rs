//! `governance` community variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{
    pseudonym_key_from_capnp, pseudonym_key_to_capnp, uuid16_from_capnp, uuid16_to_capnp,
};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

pub(super) fn write_community_meta(
    mut p: schema_pkg::community_meta_entry::Builder<'_>,
    name: Option<&str>,
    description: Option<&str>,
    icon_hash: Option<&str>,
    banner_hash: Option<&str>,
    lamport: u64,
) {
    p.set_has_name(name.is_some());
    if let Some(n) = name {
        p.set_name(n);
    }
    p.set_has_description(description.is_some());
    if let Some(d) = description {
        p.set_description(d);
    }
    p.set_has_icon_hash(icon_hash.is_some());
    if let Some(h) = icon_hash {
        p.set_icon_hash(h);
    }
    p.set_has_banner_hash(banner_hash.is_some());
    if let Some(h) = banner_hash {
        p.set_banner_hash(h);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_community_notification_default(
    mut p: schema_pkg::community_notification_default_entry::Builder<'_>,
    level: &str,
    lamport: u64,
) {
    p.set_level(level);
    p.set_lamport(lamport);
}

pub(super) fn write_mek_generation_bump(
    mut p: schema_pkg::m_e_k_generation_bump_entry::Builder<'_>,
    generation: u64,
    trigger_departed: &PseudonymKey,
    cascade_skipped: &[PseudonymKey],
    lamport: u64,
) {
    p.set_generation(generation);
    pseudonym_key_to_capnp(p.reborrow().init_trigger_departed(), trigger_departed);
    let mut list = p
        .reborrow()
        .init_cascade_skipped(len_u32(cascade_skipped.len()));
    for (i, k) in cascade_skipped.iter().enumerate() {
        pseudonym_key_to_capnp(list.reborrow().get(len_u32(i)), k);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_segment_added(
    mut p: schema_pkg::segment_added_entry::Builder<'_>,
    segment_index: u32,
    registry_key: &str,
    governance_key: &str,
    slot_range_start: u32,
    slot_range_end: u32,
    lamport: u64,
) {
    p.set_segment_index(segment_index);
    p.set_registry_key(registry_key);
    p.set_governance_key(governance_key);
    p.set_slot_range_start(slot_range_start);
    p.set_slot_range_end(slot_range_end);
    p.set_lamport(lamport);
}

pub(super) fn write_invite_created(
    mut p: schema_pkg::invite_created_entry::Builder<'_>,
    invite_id: &[u8; 16],
    code_hash: &str,
    max_uses: u32,
    expires_at: Option<u64>,
    secrets_record_key: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_invite_id(), invite_id);
    p.set_code_hash(code_hash);
    p.set_max_uses(max_uses);
    p.set_has_expires_at(expires_at.is_some());
    if let Some(t) = expires_at {
        p.set_expires_at(t);
    }
    p.set_secrets_record_key(secrets_record_key);
    p.set_lamport(lamport);
}

pub(super) fn write_invite_revoked(
    mut p: schema_pkg::invite_revoked_entry::Builder<'_>,
    invite_id: &[u8; 16],
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_invite_id(), invite_id);
    p.set_lamport(lamport);
}

pub(super) fn write_community_policy(
    mut p: schema_pkg::community_policy_entry::Builder<'_>,
    policy_text: Option<&str>,
    max_joins_per_interval: u32,
    join_interval_seconds: u32,
    lamport: u64,
) {
    p.set_has_policy_text(policy_text.is_some());
    if let Some(t) = policy_text {
        p.set_policy_text(t);
    }
    p.set_max_joins_per_interval(max_joins_per_interval);
    p.set_join_interval_seconds(join_interval_seconds);
    p.set_lamport(lamport);
}

pub(super) fn read_community_meta(
    p: schema_pkg::community_meta_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityMeta {
        name: if p.get_has_name() {
            Some(text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        description: if p.get_has_description() {
            Some(text_to_string(
                p.get_description().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        icon_hash: if p.get_has_icon_hash() {
            Some(text_to_string(
                p.get_icon_hash().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        banner_hash: if p.get_has_banner_hash() {
            Some(text_to_string(
                p.get_banner_hash().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_community_notification_default(
    p: schema_pkg::community_notification_default_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityNotificationDefault {
        level: text_to_string(p.get_level().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_mek_generation_bump(
    p: schema_pkg::m_e_k_generation_bump_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let cascade: Result<Vec<_>, ProtocolError> = p
        .get_cascade_skipped()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(pseudonym_key_from_capnp)
        .collect();
    Ok(GovernanceEntry::MEKGenerationBump {
        generation: p.get_generation(),
        trigger_departed: pseudonym_key_from_capnp(
            p.get_trigger_departed().map_err(|e| capnp_err(&e))?,
        )?,
        cascade_skipped: cascade?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_segment_added(
    p: schema_pkg::segment_added_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::SegmentAdded {
        segment_index: p.get_segment_index(),
        registry_key: text_to_string(p.get_registry_key().map_err(|e| capnp_err(&e))?)?,
        governance_key: text_to_string(p.get_governance_key().map_err(|e| capnp_err(&e))?)?,
        slot_range_start: p.get_slot_range_start(),
        slot_range_end: p.get_slot_range_end(),
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_invite_created(
    p: schema_pkg::invite_created_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::InviteCreated {
        invite_id: uuid16_from_capnp(p.get_invite_id().map_err(|e| capnp_err(&e))?)?,
        code_hash: text_to_string(p.get_code_hash().map_err(|e| capnp_err(&e))?)?,
        max_uses: p.get_max_uses(),
        expires_at: if p.get_has_expires_at() {
            Some(p.get_expires_at())
        } else {
            None
        },
        secrets_record_key: text_to_string(p.get_secrets_record_key().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_invite_revoked(
    p: schema_pkg::invite_revoked_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::InviteRevoked {
        invite_id: uuid16_from_capnp(p.get_invite_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_community_policy(
    p: schema_pkg::community_policy_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::CommunityPolicy {
        policy_text: if p.get_has_policy_text() {
            Some(text_to_string(
                p.get_policy_text().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        max_joins_per_interval: p.get_max_joins_per_interval(),
        join_interval_seconds: p.get_join_interval_seconds(),
        lamport: p.get_lamport(),
    })
}

//! `governance` moderation variant encode/decode helpers.

use super::super::sub_types::{
    channel_id_from_capnp, pseudonym_key_from_capnp, pseudonym_key_to_capnp, uuid16_from_capnp,
    uuid16_to_capnp,
};
use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::{ChannelId, PseudonymKey};

pub(super) fn write_ban_entry(
    mut p: schema_pkg::ban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_unban_entry(
    mut p: schema_pkg::unban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_lamport(lamport);
}

pub(super) fn write_timeout_entry(
    mut p: schema_pkg::timeout_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    duration_seconds: u64,
    reason: Option<&str>,
    started_at: u64,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_duration_seconds(duration_seconds);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_started_at(started_at);
    p.set_lamport(lamport);
}

pub(super) fn write_remove_timeout_entry(
    mut p: schema_pkg::remove_timeout_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_lamport(lamport);
}

pub(super) fn write_admin_delete(
    mut p: schema_pkg::admin_delete_entry::Builder<'_>,
    message_id: &[u8; 16],
    channel_id: ChannelId,
    reason: Option<&str>,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_message_id(), message_id);
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_auto_mod_rule(
    mut p: schema_pkg::auto_mod_rule_entry::Builder<'_>,
    rule_id: &[u8; 16],
    name: &str,
    enabled: bool,
    trigger_json: &str,
    action: &str,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_rule_id(), rule_id);
    p.set_name(name);
    p.set_enabled(enabled);
    p.set_trigger_json(trigger_json);
    p.set_action(action);
    p.set_lamport(lamport);
}

pub(super) fn read_ban_entry(
    p: schema_pkg::ban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::BanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_unban_entry(
    p: schema_pkg::unban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::UnbanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_timeout_entry(
    p: schema_pkg::timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::TimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        duration_seconds: p.get_duration_seconds(),
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        started_at: p.get_started_at(),
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_remove_timeout_entry(
    p: schema_pkg::remove_timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::RemoveTimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_admin_delete(
    p: schema_pkg::admin_delete_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AdminDelete {
        message_id: uuid16_from_capnp(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_auto_mod_rule(
    p: schema_pkg::auto_mod_rule_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AutoModRule {
        rule_id: uuid16_from_capnp(p.get_rule_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        enabled: p.get_enabled(),
        trigger_json: text_to_string(p.get_trigger_json().map_err(|e| capnp_err(&e))?)?,
        action: text_to_string(p.get_action().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

// ── Admission ────────────────────────────────────────────────────────

pub(super) fn read_join_requested(
    p: schema_pkg::join_requested_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::JoinRequested {
        requester: pseudonym_key_from_capnp(p.get_requester().map_err(|e| capnp_err(&e))?)?,
        display_name: text_to_string(p.get_display_name().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_member_approved(
    p: schema_pkg::member_approved_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::MemberApproved {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_member_rejected(
    p: schema_pkg::member_rejected_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::MemberRejected {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        // `has_reason` rather than an empty-string sentinel, matching
        // `BanEntry` — it keeps `Some("")` distinct from `None`.
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_admission_policy(
    p: schema_pkg::admission_policy_payload::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    use schema_pkg::AdmissionMode as Wire;
    Ok(GovernanceEntry::AdmissionPolicy {
        mode: match p.get_mode().map_err(not_in_schema)? {
            Wire::Open => AdmissionMode::Open,
            Wire::ApprovalRequired => AdmissionMode::ApprovalRequired,
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn write_join_requested(
    mut p: schema_pkg::join_requested_payload::Builder<'_>,
    requester: &PseudonymKey,
    display_name: &str,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_requester(), requester);
    p.set_display_name(display_name);
    p.set_lamport(lamport);
}

pub(super) fn write_member_approved(
    mut p: schema_pkg::member_approved_payload::Builder<'_>,
    target: &PseudonymKey,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_lamport(lamport);
}

pub(super) fn write_member_rejected(
    mut p: schema_pkg::member_rejected_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
    lamport: u64,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_lamport(lamport);
}

pub(super) fn write_admission_policy(
    mut p: schema_pkg::admission_policy_payload::Builder<'_>,
    mode: AdmissionMode,
    lamport: u64,
) {
    use schema_pkg::AdmissionMode as Wire;
    p.set_mode(match mode {
        AdmissionMode::Open => Wire::Open,
        AdmissionMode::ApprovalRequired => Wire::ApprovalRequired,
    });
    p.set_lamport(lamport);
}

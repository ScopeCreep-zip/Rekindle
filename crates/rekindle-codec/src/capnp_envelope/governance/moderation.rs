//! `governance` moderation variant encode/decode helpers.

use super::super::sub_types::{
    channel_id_from_capnp, pseudonym_key_from_capnp, pseudonym_key_to_capnp, uuid16_from_capnp,
    uuid16_to_capnp,
};
use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::CodecError;
use rekindle_types::governance::{AutoModAction, GovernanceEntry, ModReportTarget};
use rekindle_types::id::{ChannelId, PseudonymKey};

pub(super) fn write_ban_entry(
    mut p: schema_pkg::ban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
}

pub(super) fn write_unban_entry(
    mut p: schema_pkg::unban_entry_payload::Builder<'_>,
    target: &PseudonymKey,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
}

pub(super) fn write_timeout_entry(
    mut p: schema_pkg::timeout_entry_payload::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::TimeoutEntry {
        target,
        duration_seconds,
        reason,
        started_at,
    } = e
    else {
        unreachable!("write_timeout_entry: variant mismatch")
    };
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_duration_seconds(*duration_seconds);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
    p.set_started_at(*started_at);
}

pub(super) fn write_remove_timeout_entry(
    mut p: schema_pkg::remove_timeout_entry_payload::Builder<'_>,
    target: &PseudonymKey,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
}

pub(super) fn write_admin_delete(
    mut p: schema_pkg::admin_delete_entry::Builder<'_>,
    message_id: &[u8; 16],
    channel_id: ChannelId,
    reason: Option<&str>,
) {
    uuid16_to_capnp(p.reborrow().init_message_id(), message_id);
    uuid16_to_capnp(p.reborrow().init_channel_id(), &channel_id.0);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
}

pub(super) fn write_auto_mod_rule(
    mut p: schema_pkg::auto_mod_rule_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::AutoModRule {
        rule_id,
        name,
        enabled,
        trigger_json,
        action,
    } = e
    else {
        unreachable!("write_auto_mod_rule: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_rule_id(), rule_id);
    p.set_name(name);
    p.set_enabled(*enabled);
    p.set_trigger_json(trigger_json);
    p.set_action_kind(match action {
        AutoModAction::BlockLocally => schema_pkg::AutoModActionKind::BlockLocally,
        AutoModAction::BlurContent => schema_pkg::AutoModActionKind::BlurContent,
        AutoModAction::AlertModerators => schema_pkg::AutoModActionKind::AlertModerators,
    });
}

pub(super) fn read_ban_entry(
    p: schema_pkg::ban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::BanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn read_unban_entry(
    p: schema_pkg::unban_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::UnbanEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_timeout_entry(
    p: schema_pkg::timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::TimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        duration_seconds: p.get_duration_seconds(),
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        started_at: p.get_started_at(),
    })
}

pub(super) fn read_remove_timeout_entry(
    p: schema_pkg::remove_timeout_entry_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::RemoveTimeoutEntry {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_admin_delete(
    p: schema_pkg::admin_delete_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::AdminDelete {
        message_id: uuid16_from_capnp(p.get_message_id().map_err(|e| capnp_err(&e))?)?,
        channel_id: channel_id_from_capnp(p.get_channel_id().map_err(|e| capnp_err(&e))?)?,
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn read_auto_mod_rule(
    p: schema_pkg::auto_mod_rule_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::AutoModRule {
        rule_id: uuid16_from_capnp(p.get_rule_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        enabled: p.get_enabled(),
        trigger_json: text_to_string(p.get_trigger_json().map_err(|e| capnp_err(&e))?)?,
        action: match p.get_action_kind().map_err(not_in_schema)? {
            schema_pkg::AutoModActionKind::BlockLocally => AutoModAction::BlockLocally,
            schema_pkg::AutoModActionKind::BlurContent => AutoModAction::BlurContent,
            schema_pkg::AutoModActionKind::AlertModerators => AutoModAction::AlertModerators,
        },
    })
}

// ── Admission ────────────────────────────────────────────────────────

pub(super) fn read_member_approved(
    p: schema_pkg::member_approved_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::MemberApproved {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
    })
}

pub(super) fn read_member_rejected(
    p: schema_pkg::member_rejected_payload::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::MemberRejected {
        target: pseudonym_key_from_capnp(p.get_target().map_err(|e| capnp_err(&e))?)?,
        // `has_reason` rather than an empty-string sentinel, matching
        // `BanEntry` — it keeps `Some("")` distinct from `None`.
        reason: if p.get_has_reason() {
            Some(text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
    })
}

pub(super) fn write_member_approved(
    mut p: schema_pkg::member_approved_payload::Builder<'_>,
    target: &PseudonymKey,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
}

pub(super) fn write_member_rejected(
    mut p: schema_pkg::member_rejected_payload::Builder<'_>,
    target: &PseudonymKey,
    reason: Option<&str>,
) {
    pseudonym_key_to_capnp(p.reborrow().init_target(), target);
    p.set_has_reason(reason.is_some());
    if let Some(r) = reason {
        p.set_reason(r);
    }
}

pub(super) fn write_mod_report(
    mut p: schema_pkg::mod_report_entry::Builder<'_>,
    report_id: &[u8; 16],
    target: &ModReportTarget,
    reason: &str,
) {
    uuid16_to_capnp(p.reborrow().init_report_id(), report_id);
    match target {
        ModReportTarget::Message {
            channel_id,
            message_id,
        } => {
            let mut m = p.reborrow().init_target().init_message();
            uuid16_to_capnp(m.reborrow().init_channel_id(), &channel_id.0);
            m.set_message_id(message_id);
        }
        ModReportTarget::Member { pseudonym } => {
            pseudonym_key_to_capnp(p.reborrow().init_target().init_member(), pseudonym);
        }
    }
    p.set_reason(reason);
}

pub(super) fn read_mod_report(
    p: schema_pkg::mod_report_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    use schema_pkg::mod_report_entry::target::Which;
    let target = match p.get_target().which().map_err(not_in_schema)? {
        Which::Message(m) => ModReportTarget::Message {
            channel_id: channel_id_from_capnp(m.get_channel_id().map_err(|e| capnp_err(&e))?)?,
            message_id: text_to_string(m.get_message_id().map_err(|e| capnp_err(&e))?)?,
        },
        Which::Member(m) => ModReportTarget::Member {
            pseudonym: pseudonym_key_from_capnp(m.map_err(|e| capnp_err(&e))?)?,
        },
    };
    Ok(GovernanceEntry::ModReport {
        report_id: uuid16_from_capnp(p.get_report_id().map_err(|e| capnp_err(&e))?)?,
        target,
        reason: text_to_string(p.get_reason().map_err(|e| capnp_err(&e))?)?,
    })
}

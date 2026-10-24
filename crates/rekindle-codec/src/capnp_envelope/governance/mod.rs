//! `GovernanceEntry` union encoder / decoder.
//!
//! Mirrors `rekindle_types::governance::GovernanceEntry` against the
//! schema in `schemas/community_governance.capnp`. Each variant has
//! its own `write_<name>` / `read_<name>` helper so the dispatcher
//! `write_governance_entry` / `read_governance_entry` stay below the
//! workspace `too-many-lines` threshold and individual variant logic
//! is independently reviewable.

use crate::capnp_codec::{bounds, capnp_err, pack, unpack_bounded};
use crate::community_governance_capnp::governance_entry as schema;
use crate::error::CodecError;
use rekindle_types::governance::GovernanceEntry;

/// A `write_<name>` for a payload that is a target and an optional
/// reason (ban, rejection, removal): one definition, expanded in the
/// variant's module.
macro_rules! target_reason_writer {
    ($name:ident, $payload:ident) => {
        pub(super) fn $name(
            mut p: crate::community_governance_capnp::$payload::Builder<'_>,
            target: &rekindle_types::id::PseudonymKey,
            reason: Option<&str>,
        ) {
            super::super::sub_types::pseudonym_key_to_capnp(p.reborrow().init_target(), target);
            p.set_has_reason(reason.is_some());
            if let Some(r) = reason {
                p.set_reason(r);
            }
        }
    };
}

mod channels;
mod community;
mod dispatch_write;
mod events;
mod expression;
mod membership;
mod moderation;
mod onboarding;
mod roles;
mod shared;

use channels::{
    read_category_archived, read_category_created, read_category_updated, read_channel_archived,
    read_channel_created, read_channel_segment_linked, read_channel_updated,
    read_permission_overwrite,
};
use community::{
    read_community_meta, read_community_notification_default, read_community_policy,
    read_invite_created, read_invite_revoked, read_mek_generation_bump, read_segment_added,
};
pub(crate) use dispatch_write::write_governance_entry;
use events::{
    read_event_archived, read_event_created, read_event_rsvp, read_thread_archived,
    read_thread_created, read_thread_unarchived,
};
use expression::{read_attachment_pinned, read_expression_added, read_expression_removed};
use membership::{read_member_joined, read_member_record_moved, read_member_removed};
use moderation::{
    read_admin_delete, read_auto_mod_rule, read_ban_entry, read_member_approved,
    read_member_rejected, read_mod_report, read_remove_timeout_entry, read_timeout_entry,
    read_unban_entry,
};
use onboarding::{read_onboarding_config, read_welcome_screen};
use roles::{
    read_role_archived, read_role_assignment, read_role_definition, read_role_unassignment,
};

/// Encode a single `GovernanceEntry` to packed Cap'n Proto bytes
/// (used for direct serialization, e.g. SMPL subkey writes).
pub fn encode_governance_entry(entry: &GovernanceEntry) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let root = builder.init_root::<schema::Builder<'_>>();
    write_governance_entry(root, entry);
    pack(&builder)
}

/// Decode a single `GovernanceEntry` from packed Cap'n Proto bytes.
pub fn decode_governance_entry(bytes: &[u8]) -> Result<GovernanceEntry, CodecError> {
    let reader = unpack_bounded(bytes, bounds::GOVERNANCE_ENTRY_MAX_WORDS)?;
    let root = reader
        .get_root::<schema::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read_governance_entry(root)
}

pub(crate) fn read_governance_entry(r: schema::Reader<'_>) -> Result<GovernanceEntry, CodecError> {
    use schema::Which;
    let which = r
        .which()
        .map_err(|e| CodecError::UnknownGovernanceArm(format!("governance entry arm {}", e.0)))?;
    match which {
        Which::ObsoleteJoinRequested(_) => Err(CodecError::UnknownGovernanceArm(
            "retired joinRequested (@34)".into(),
        )),
        Which::ObsoleteAdmissionPolicy(_) => Err(CodecError::UnknownGovernanceArm(
            "retired admissionPolicy (@37)".into(),
        )),
        Which::MemberJoined(p) => read_member_joined(p.map_err(|e| capnp_err(&e))?),
        Which::MemberRecordMoved(p) => read_member_record_moved(p.map_err(|e| capnp_err(&e))?),
        Which::MemberLeft(_) => Ok(GovernanceEntry::MemberLeft {}),
        Which::MemberRemoved(p) => read_member_removed(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelCreated(p) => read_channel_created(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelArchived(p) => read_channel_archived(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelUpdated(p) => read_channel_updated(p.map_err(|e| capnp_err(&e))?),
        Which::RoleDefinition(p) => read_role_definition(p.map_err(|e| capnp_err(&e))?),
        Which::RoleAssignment(p) => read_role_assignment(p.map_err(|e| capnp_err(&e))?),
        Which::RoleUnassignment(p) => read_role_unassignment(p.map_err(|e| capnp_err(&e))?),
        Which::BanEntry(p) => read_ban_entry(p.map_err(|e| capnp_err(&e))?),
        Which::UnbanEntry(p) => read_unban_entry(p.map_err(|e| capnp_err(&e))?),
        Which::MemberApproved(p) => read_member_approved(p.map_err(|e| capnp_err(&e))?),
        Which::MemberRejected(p) => read_member_rejected(p.map_err(|e| capnp_err(&e))?),
        Which::TimeoutEntry(p) => read_timeout_entry(p.map_err(|e| capnp_err(&e))?),
        Which::RemoveTimeoutEntry(p) => read_remove_timeout_entry(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityMeta(p) => read_community_meta(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityNotificationDefault(p) => {
            read_community_notification_default(p.map_err(|e| capnp_err(&e))?)
        }
        Which::MekGenerationBump(p) => read_mek_generation_bump(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryCreated(p) => read_category_created(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryArchived(p) => read_category_archived(p.map_err(|e| capnp_err(&e))?),
        Which::PermissionOverwrite(p) => read_permission_overwrite(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadCreated(p) => read_thread_created(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadArchived(p) => read_thread_archived(p.map_err(|e| capnp_err(&e))?),
        Which::ThreadUnarchived(p) => read_thread_unarchived(p.map_err(|e| capnp_err(&e))?),
        Which::EventCreated(p) => read_event_created(p.map_err(|e| capnp_err(&e))?),
        Which::ExpressionAdded(p) => read_expression_added(p.map_err(|e| capnp_err(&e))?),
        Which::ExpressionRemoved(p) => read_expression_removed(p.map_err(|e| capnp_err(&e))?),
        Which::EventArchived(p) => read_event_archived(p.map_err(|e| capnp_err(&e))?),
        Which::EventRsvp(p) => read_event_rsvp(p.map_err(|e| capnp_err(&e))?),
        Which::ModReport(p) => read_mod_report(p.map_err(|e| capnp_err(&e))?),
        Which::OnboardingConfig(p) => read_onboarding_config(p.map_err(|e| capnp_err(&e))?),
        Which::WelcomeScreen(p) => read_welcome_screen(p.map_err(|e| capnp_err(&e))?),
        Which::AdminDelete(p) => read_admin_delete(p.map_err(|e| capnp_err(&e))?),
        Which::ChannelSegmentLinked(p) => {
            read_channel_segment_linked(p.map_err(|e| capnp_err(&e))?)
        }
        Which::SegmentAdded(p) => read_segment_added(p.map_err(|e| capnp_err(&e))?),
        Which::AutoModRule(p) => read_auto_mod_rule(p.map_err(|e| capnp_err(&e))?),
        Which::RoleArchived(p) => read_role_archived(p.map_err(|e| capnp_err(&e))?),
        Which::CategoryUpdated(p) => read_category_updated(p.map_err(|e| capnp_err(&e))?),
        Which::InviteCreated(p) => read_invite_created(p.map_err(|e| capnp_err(&e))?),
        Which::InviteRevoked(p) => read_invite_revoked(p.map_err(|e| capnp_err(&e))?),
        Which::AttachmentPinned(p) => read_attachment_pinned(p.map_err(|e| capnp_err(&e))?),
        Which::CommunityPolicy(p) => read_community_policy(p.map_err(|e| capnp_err(&e))?),
    }
}

#[cfg(test)]
mod admission_wire_tests {
    //! Round-trips for the admission variants.
    //!
    //! These are not ceremony. `dispatch_write` routes variants through
    //! two halves guarded by `unreachable!()`, so a variant that is
    //! declared but not wired **compiles fine and panics at runtime**.
    //! Only an actual encode→decode proves the wiring exists.

    use rekindle_types::governance::GovernanceEntry;
    use rekindle_types::id::PseudonymKey;

    use super::{decode_governance_entry, encode_governance_entry};

    fn pseudo(b: u8) -> PseudonymKey {
        PseudonymKey([b; 32])
    }

    fn round_trip(entry: &GovernanceEntry) {
        let decoded = decode_governance_entry(&encode_governance_entry(entry))
            .expect("admission entry must decode");
        assert_eq!(&decoded, entry);
    }

    #[test]
    fn member_approved_round_trips() {
        round_trip(&GovernanceEntry::MemberApproved { target: pseudo(2) });
    }

    #[test]
    fn member_rejected_round_trips_with_and_without_reason() {
        round_trip(&GovernanceEntry::MemberRejected {
            target: pseudo(3),
            reason: Some("spam".to_string()),
        });
        // `None` must not come back as `Some("")` — that is why the wire
        // carries an explicit `hasReason`, matching `BanEntry`.
        round_trip(&GovernanceEntry::MemberRejected {
            target: pseudo(3),
            reason: None,
        });
    }

    #[test]
    fn event_rsvp_round_trips_every_status() {
        use rekindle_types::event::RsvpStatus;
        use rekindle_types::id::EventId;
        for status in [
            RsvpStatus::Going,
            RsvpStatus::Interested,
            RsvpStatus::Declined,
        ] {
            round_trip(&GovernanceEntry::EventRsvp {
                event_id: EventId([0x5A; 16]),
                status,
            });
        }
    }

    #[test]
    fn mod_report_round_trips_both_targets() {
        use rekindle_types::governance::ModReportTarget;
        use rekindle_types::id::{ChannelId, PseudonymKey};
        round_trip(&GovernanceEntry::ModReport {
            report_id: [0x11; 16],
            target: ModReportTarget::Message {
                channel_id: ChannelId([0x22; 16]),
                message_id: "m-1".into(),
            },
            reason: "alert_moderators: slurs".into(),
        });
        round_trip(&GovernanceEntry::ModReport {
            report_id: [0x33; 16],
            target: ModReportTarget::Member {
                pseudonym: PseudonymKey([0x44; 32]),
            },
            reason: "spam".into(),
        });
    }

    #[test]
    fn membership_ops_round_trip() {
        use rekindle_types::governance::InviteProof;
        for proof in [
            None,
            Some(InviteProof {
                code_hash: "code".into(),
                sig: vec![9; 64],
            }),
        ] {
            round_trip(&GovernanceEntry::MemberJoined {
                member_record: "VLD0:m".into(),
                member_record_owner: "owner".into(),
                invite_proof: proof,
            });
        }
        round_trip(&GovernanceEntry::MemberRecordMoved {
            member_record: "VLD0:n".into(),
            member_record_owner: "owner2".into(),
        });
        round_trip(&GovernanceEntry::MemberLeft {});
        for reason in [None, Some(String::new()), Some("spam".into())] {
            round_trip(&GovernanceEntry::MemberRemoved {
                target: pseudo(4),
                reason,
            });
        }
    }

    #[test]
    fn retired_arms_decode_to_unknown() {
        use crate::community_governance_capnp::governance_entry as schema;
        use crate::error::CodecError;
        let mut builder = capnp::message::Builder::new_default();
        let mut root = builder.init_root::<schema::Builder<'_>>();
        root.reborrow().init_obsolete_admission_policy();
        let bytes = crate::capnp_codec::pack(&builder);
        assert!(matches!(
            decode_governance_entry(&bytes),
            Err(CodecError::UnknownGovernanceArm(_))
        ));
        let mut builder = capnp::message::Builder::new_default();
        let root = builder.init_root::<schema::Builder<'_>>();
        root.init_obsolete_join_requested();
        let bytes = crate::capnp_codec::pack(&builder);
        assert!(matches!(
            decode_governance_entry(&bytes),
            Err(CodecError::UnknownGovernanceArm(_))
        ));
    }
}

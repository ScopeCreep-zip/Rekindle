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
            owner_proof: vec![7; 64],
            first_device: None,
        });
    }
    round_trip(&GovernanceEntry::MemberRecordMoved {
        member_record: "VLD0:n".into(),
        member_record_owner: "owner2".into(),
        owner_proof: vec![6; 64],
    });
    for heads in [
        Vec::new(),
        vec![rekindle_types::governance::SlotHead {
            record_key: "VLD0:channel".into(),
            seq: 3,
            digest: [3; 32],
        }],
    ] {
        round_trip(&GovernanceEntry::MemberLeft { heads });
    }
    for reason in [None, Some(String::new()), Some("spam".into())] {
        round_trip(&GovernanceEntry::MemberRemoved {
            target: pseudo(4),
            reason,
            heads: vec![rekindle_types::governance::SlotHead {
                record_key: "VLD0:channel".into(),
                seq: 7,
                digest: [7; 32],
            }],
            target_seq_frontier: 5,
        });
    }
}

/// A `MemberLeft` that states no slot heads (the list left out, not
/// empty) is refused: readers keep a leaver's posts from history only
/// by its heads (A/B finding R3-8), and no build wrote the bare shape.
#[test]
fn a_member_left_without_its_heads_is_refused() {
    use crate::community_governance_capnp::governance_entry as schema;
    use crate::error::CodecError;
    let mut builder = capnp::message::Builder::new_default();
    let root = builder.init_root::<schema::Builder<'_>>();
    root.init_member_left();
    let bytes = crate::capnp_codec::pack(&builder);
    assert!(matches!(
        decode_governance_entry(&bytes),
        Err(CodecError::Deserialization(_))
    ));
}

/// A slot head without its 32-byte digest is refused (plan S9): a
/// departed member's posts are kept only from exactly the committed
/// value, and no build wrote a head without one.
#[test]
fn a_slot_head_without_its_digest_is_refused() {
    use crate::community_governance_capnp::governance_entry as schema;
    use crate::error::CodecError;
    for digest in [None, Some(&[9u8; 31][..])] {
        let mut builder = capnp::message::Builder::new_default();
        let root = builder.init_root::<schema::Builder<'_>>();
        let mut head = root.init_member_left().init_heads(1).get(0);
        head.set_record_key("VLD0:channel");
        head.set_seq(3);
        if let Some(digest) = digest {
            head.set_digest(digest);
        }
        let bytes = crate::capnp_codec::pack(&builder);
        assert!(matches!(
            decode_governance_entry(&bytes),
            Err(CodecError::Deserialization(_))
        ));
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

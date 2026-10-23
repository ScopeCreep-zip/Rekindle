//! Op wire-format tests: every body arm round-trips, the canonical
//! unsigned bytes are stable, unknown arms are reported, and the deps
//! bound holds at its maximum.

use rekindle_types::event::RsvpStatus;
use rekindle_types::governance::{
    AutoModAction, GovernanceEntry, GovernanceOp, GuideStep, InviteProof, ModReportTarget, OpHash,
    OpRef, WelcomeChannel,
};
use rekindle_types::id::{CategoryId, ChannelId, EventId, PseudonymKey, RoleId, ThreadId};

use super::{
    decode_op, encode_op, op_unsigned_bytes, GOVERNANCE_OP_MAX_BYTES, GOVERNANCE_OP_MAX_DEPS,
};
use crate::CodecError;

fn pk(b: u8) -> PseudonymKey {
    PseudonymKey([b; 32])
}

fn s(v: &str) -> String {
    v.to_owned()
}

/// One value of every live `GovernanceEntry` arm.
fn every_body() -> Vec<GovernanceEntry> {
    let mut all = channel_and_role_bodies();
    all.extend(member_bodies());
    all.extend(structure_bodies());
    all.extend(moderation_and_policy_bodies());
    all
}

fn channel_and_role_bodies() -> Vec<GovernanceEntry> {
    use GovernanceEntry as E;
    let ch = ChannelId([0x21; 16]);
    vec![
        E::ChannelCreated {
            channel_id: ch,
            name: s("general"),
            channel_type: s("text"),
            record_key: s("VLD0:c"),
            category_id: Some(CategoryId([3; 16])),
            position: 2,
            parent_voice_channel_id: None,
        },
        E::ChannelArchived { channel_id: ch },
        E::ChannelUpdated {
            channel_id: ch,
            name: Some(s("n")),
            topic: Some(s("t")),
            forum_tags: Some(vec![s("a")]),
            position: Some(1),
            slowmode_seconds: Some(5),
            nsfw: Some(true),
            category_id: Some(Some(CategoryId([4; 16]))),
        },
        E::RoleDefinition {
            role_id: RoleId([5; 16]),
            name: s("mod"),
            permissions: 0xff,
            position: 3,
            color: 0xff00_00ff,
            hoist: true,
            mentionable: false,
            self_assignable: true,
            exclusion_group: Some(s("g")),
        },
        E::RoleAssignment {
            target: pk(1),
            role_id: RoleId([5; 16]),
        },
        E::RoleUnassignment {
            target: pk(1),
            role_id: RoleId([5; 16]),
        },
        E::BanEntry {
            target: pk(2),
            reason: Some(s("spam")),
        },
        E::UnbanEntry { target: pk(2) },
    ]
}

fn member_bodies() -> Vec<GovernanceEntry> {
    use GovernanceEntry as E;
    vec![
        E::MemberJoined {
            member_record: s("VLD0:m"),
            member_record_owner: s("own"),
            invite_proof: Some(InviteProof {
                code_hash: s("h"),
                sig: vec![9; 64],
            }),
        },
        E::MemberRecordMoved {
            member_record: s("VLD0:n"),
            member_record_owner: s("own2"),
        },
        E::MemberLeft {},
        E::MemberRemoved {
            target: pk(3),
            reason: None,
        },
        E::MemberApproved { target: pk(4) },
        E::MemberRejected {
            target: pk(4),
            reason: Some(s("no")),
        },
        E::TimeoutEntry {
            target: pk(5),
            duration_seconds: 60,
            reason: Some(s("calm")),
            started_at: 1_000,
        },
        E::RemoveTimeoutEntry { target: pk(5) },
        E::CommunityMeta {
            name: Some(s("c")),
            description: Some(s("d")),
            icon_hash: None,
            banner_hash: Some(s("b")),
        },
        E::CommunityNotificationDefault {
            level: s("mentions"),
        },
        E::MEKGenerationBump {
            generation: 7,
            trigger_departed: pk(6),
            cascade_skipped: vec![pk(7)],
        },
    ]
}

fn structure_bodies() -> Vec<GovernanceEntry> {
    use GovernanceEntry as E;
    let ch = ChannelId([0x21; 16]);
    vec![
        E::CategoryCreated {
            category_id: CategoryId([8; 16]),
            name: s("cat"),
            position: 0,
        },
        E::CategoryArchived {
            category_id: CategoryId([8; 16]),
        },
        E::PermissionOverwrite {
            channel_id: ch,
            target_type: s("role"),
            target_id: s("r"),
            allow: 1,
            deny: 2,
        },
        E::ThreadCreated {
            thread_id: ThreadId([9; 16]),
            parent_channel_id: ch,
            name: s("th"),
            thread_type: s("public"),
            record_key: Some(s("VLD0:t")),
            invited: vec![pk(8)],
            forum_tag: None,
            auto_archive_seconds: 3_600,
        },
        E::ThreadArchived {
            thread_id: ThreadId([9; 16]),
        },
        E::ThreadUnarchived {
            thread_id: ThreadId([9; 16]),
        },
        E::EventCreated {
            event_id: EventId([10; 16]),
            name: s("lan"),
            description: Some(s("d")),
            start_time: 10,
            end_time: Some(20),
            channel_id: Some(ch),
            cover_image_ref: None,
            creator_pseudonym: Some(pk(9)),
            recurrence: None,
            location: None,
            status: None,
        },
        E::ExpressionAdded {
            expression_id: [11; 16],
            name: s("fire"),
            kind: s("emoji"),
            content_hash: s("ch"),
            attachment: None,
            animated: false,
            tags: vec![s("hot")],
            sound_meta: None,
            creator_pseudonym: Some(pk(10)),
            created_at: Some(5),
            available_to_peers: Some(true),
        },
        E::ExpressionRemoved {
            expression_id: [11; 16],
        },
        E::EventArchived {
            event_id: EventId([10; 16]),
        },
        E::EventRsvp {
            event_id: EventId([10; 16]),
            status: RsvpStatus::Going,
        },
    ]
}

fn moderation_and_policy_bodies() -> Vec<GovernanceEntry> {
    use GovernanceEntry as E;
    let ch = ChannelId([0x21; 16]);
    vec![
        E::ModReport {
            report_id: [12; 16],
            target: ModReportTarget::Member { pseudonym: pk(11) },
            reason: s("spam"),
        },
        E::OnboardingConfig {
            enabled: true,
            mode: s("default"),
            default_channels: vec![ch],
            questions: Vec::new(),
            welcome_message: Some(s("hi")),
            guide_steps: vec![GuideStep {
                title: s("t"),
                description: s("d"),
                channel_id: Some(ch),
                emoji: None,
            }],
        },
        E::WelcomeScreen {
            description: s("w"),
            channels: vec![WelcomeChannel {
                channel_id: ch,
                description: s("d"),
                emoji: Some(s("e")),
            }],
        },
        E::AdminDelete {
            message_id: [13; 16],
            channel_id: ch,
            reason: None,
        },
        E::ChannelSegmentLinked {
            channel_id: ch,
            segment_index: 1,
            record_key: s("VLD0:s"),
        },
        E::SegmentAdded {
            segment_index: 1,
            registry_key: s("VLD0:r"),
            governance_key: s("VLD0:g"),
            slot_range_start: 0,
            slot_range_end: 255,
        },
        E::AutoModRule {
            rule_id: [14; 16],
            name: s("r"),
            enabled: true,
            trigger_json: s("{}"),
            action: AutoModAction::BlurContent,
        },
        E::RoleArchived {
            role_id: RoleId([5; 16]),
        },
        E::CategoryUpdated {
            category_id: CategoryId([8; 16]),
            name: Some(s("c2")),
            position: None,
        },
        E::InviteCreated {
            invite_id: [15; 16],
            code_hash: s("h"),
            max_uses: 3,
            expires_at: Some(99),
            secrets_record_key: s("VLD0:x"),
        },
        E::InviteRevoked {
            invite_id: [15; 16],
        },
        E::AttachmentPinned {
            attachment_id: [16; 16],
            pinned: true,
        },
        E::CommunityPolicy {
            policy_text: Some(s("be kind")),
            max_joins_per_interval: 10,
            join_interval_seconds: 60,
        },
    ]
}

/// The arm's index: exhaustive, so a new arm fails to compile here until
/// it has a value above.
fn arm_index(entry: &GovernanceEntry) -> usize {
    use GovernanceEntry as E;
    match entry {
        E::ChannelCreated { .. } => 0,
        E::ChannelArchived { .. } => 1,
        E::ChannelUpdated { .. } => 2,
        E::RoleDefinition { .. } => 3,
        E::RoleAssignment { .. } => 4,
        E::RoleUnassignment { .. } => 5,
        E::BanEntry { .. } => 6,
        E::UnbanEntry { .. } => 7,
        E::MemberJoined { .. } => 8,
        E::MemberRecordMoved { .. } => 9,
        E::MemberLeft { .. } => 10,
        E::MemberRemoved { .. } => 11,
        E::MemberApproved { .. } => 12,
        E::MemberRejected { .. } => 13,
        E::TimeoutEntry { .. } => 14,
        E::RemoveTimeoutEntry { .. } => 15,
        E::CommunityMeta { .. } => 16,
        E::CommunityNotificationDefault { .. } => 17,
        E::MEKGenerationBump { .. } => 18,
        E::CategoryCreated { .. } => 19,
        E::CategoryArchived { .. } => 20,
        E::PermissionOverwrite { .. } => 21,
        E::ThreadCreated { .. } => 22,
        E::ThreadArchived { .. } => 23,
        E::ThreadUnarchived { .. } => 24,
        E::EventCreated { .. } => 25,
        E::ExpressionAdded { .. } => 26,
        E::ExpressionRemoved { .. } => 27,
        E::EventArchived { .. } => 28,
        E::EventRsvp { .. } => 29,
        E::ModReport { .. } => 30,
        E::OnboardingConfig { .. } => 31,
        E::WelcomeScreen { .. } => 32,
        E::AdminDelete { .. } => 33,
        E::ChannelSegmentLinked { .. } => 34,
        E::SegmentAdded { .. } => 35,
        E::AutoModRule { .. } => 36,
        E::RoleArchived { .. } => 37,
        E::CategoryUpdated { .. } => 38,
        E::InviteCreated { .. } => 39,
        E::InviteRevoked { .. } => 40,
        E::AttachmentPinned { .. } => 41,
        E::CommunityPolicy { .. } => 42,
    }
}

fn op_ref(b: u8) -> OpRef {
    OpRef {
        op_hash: OpHash([b; 32]),
        author: pk(b),
        // A full-length DHT key: `VLD0:` and 43 base64url characters.
        author_record: format!("VLD0:{}", "A".repeat(43)),
        author_seq: u64::MAX - u64::from(b),
    }
}

fn op(body: GovernanceEntry, deps: Vec<OpRef>) -> GovernanceOp {
    GovernanceOp {
        author: pk(0xA0),
        seq: 4,
        prev: Some(OpHash([0xB0; 32])),
        deps,
        issued_at: 1_700_000_000,
        body,
        sig: [0xC0; 64],
    }
}

#[test]
fn every_body_arm_round_trips() {
    let bodies = every_body();
    let arms: std::collections::BTreeSet<usize> = bodies.iter().map(arm_index).collect();
    assert_eq!(arms.len(), bodies.len(), "one value per arm");
    assert_eq!(arms.len(), 43, "every live arm");
    for body in bodies {
        let original = op(body, vec![op_ref(1), op_ref(2)]);
        let decoded = decode_op(&encode_op(&original).unwrap()).unwrap();
        assert_eq!(decoded.op, original);
        assert_eq!(decoded.unsigned, op_unsigned_bytes(&original));
    }
}

#[test]
fn seq_zero_without_prev_round_trips() {
    let mut original = op(GovernanceEntry::MemberLeft {}, Vec::new());
    original.seq = 0;
    original.prev = None;
    assert_eq!(
        decode_op(&encode_op(&original).unwrap()).unwrap().op,
        original
    );
}

#[test]
fn canonical_bytes_are_stable() {
    for body in every_body() {
        let first = decode_op(&encode_op(&op(body, vec![op_ref(3)])).unwrap()).unwrap();
        let second = decode_op(&encode_op(&first.op).unwrap()).unwrap();
        assert_eq!(first.unsigned, second.unsigned);
        assert_eq!(first.op, second.op);
        // The unsigned form carries no signature and is not the signed form.
        let mut resigned = first.op.clone();
        resigned.sig = [0x11; 64];
        assert_eq!(op_unsigned_bytes(&resigned), first.unsigned);
        assert_ne!(encode_op(&first.op).unwrap(), first.unsigned);
    }
}

#[test]
fn unknown_and_retired_body_arms_are_reported() {
    use crate::governance_op_capnp::governance_op;
    let mut builder = capnp::message::Builder::new_default();
    let mut root = builder.init_root::<governance_op::Builder<'_>>();
    root.set_author(&[1; 32]);
    root.set_sig(&[2; 64]);
    root.reborrow().init_body().init_obsolete_join_requested();
    let bytes = crate::capnp_codec::pack(&builder);
    assert!(matches!(
        decode_op(&bytes),
        Err(CodecError::UnknownGovernanceArm(_))
    ));

    // An arm number beyond the schema: overwrite the body's union
    // discriminant, the first 16 bits of its data section.
    let mut builder = capnp::message::Builder::new_default();
    let mut root = builder.init_root::<governance_op::Builder<'_>>();
    root.set_author(&[1; 32]);
    root.set_sig(&[2; 64]);
    root.reborrow().init_body().init_member_left();
    let mut raw = capnp::serialize::write_message_to_words(&builder);
    let body = find_body_discriminant(&raw);
    raw[body..body + 2].copy_from_slice(&0x7fffu16.to_le_bytes());
    let message =
        capnp::serialize::read_message(raw.as_slice(), capnp::message::ReaderOptions::new())
            .unwrap();
    let mut repacked = capnp::message::Builder::new_default();
    repacked
        .set_root(message.get_root::<governance_op::Reader<'_>>().unwrap())
        .unwrap();
    assert!(matches!(
        decode_op(&crate::capnp_codec::pack(&repacked)),
        Err(CodecError::UnknownGovernanceArm(_))
    ));
}

/// Byte offset of the body struct's data section in an unpacked
/// single-segment message, from the root's `body` pointer (pointer 4:
/// author, prev, deps, body, sig in field order of the pointer section).
fn find_body_discriminant(raw: &[u8]) -> usize {
    let offset = |i: usize| {
        let v = i32::from_le_bytes(raw[i * 8..i * 8 + 4].try_into().unwrap()) >> 2;
        usize::try_from(v).unwrap()
    };
    // Word 0 is the segment table; word 1 the root pointer.
    let root_start = 2 + offset(1);
    let data_words = usize::from(u16::from_le_bytes(raw[12..14].try_into().unwrap()));
    // Pointers: author 0, prev 1, deps 2, body 3, sig 4.
    let ptr_at = root_start + data_words + 3;
    (ptr_at + 1 + offset(ptr_at)) * 8
}

#[test]
fn max_deps_fit_the_subkey_and_one_more_is_refused() {
    let deps: Vec<OpRef> = (0..GOVERNANCE_OP_MAX_DEPS)
        .map(|i| op_ref(u8::try_from(i).unwrap()))
        .collect();
    let body = GovernanceEntry::CommunityPolicy {
        policy_text: Some("p".repeat(4_096)),
        max_joins_per_interval: 1,
        join_interval_seconds: 1,
    };
    let full = op(body, deps);
    let bytes = encode_op(&full).unwrap();
    assert!(
        bytes.len() <= GOVERNANCE_OP_MAX_BYTES,
        "{} bytes",
        bytes.len()
    );
    assert_eq!(decode_op(&bytes).unwrap().op, full);

    let mut over = full;
    over.deps.push(op_ref(0xFF));
    assert!(encode_op(&over).is_err());
    // A sender that skips the encoder's check is refused on decode.
    let mut unchecked = super::encode(&over, true);
    assert!(decode_op(&unchecked).is_err());
    unchecked.resize(GOVERNANCE_OP_MAX_BYTES + 1, 0);
    assert!(decode_op(&unchecked).is_err());
}

#[test]
fn a_full_frontier_round_trips_under_the_subkey_cap() {
    use rekindle_types::dht_layout::member_record::{MAX_FRONTIER_REFS, MAX_SUBKEY_VALUE_LEN};
    let heads: Vec<OpRef> = (0..MAX_FRONTIER_REFS)
        .map(|i| OpRef {
            op_hash: OpHash([0xA5; 32]),
            author: pk(u8::try_from(i).unwrap()),
            author_record: "VLD0:".to_string() + &"x".repeat(43) + ":" + &"y".repeat(43),
            author_seq: u64::MAX - i as u64,
        })
        .collect();
    let bytes = super::encode_frontier(&heads).unwrap();
    assert!(bytes.len() <= MAX_SUBKEY_VALUE_LEN);
    assert_eq!(super::decode_frontier(&bytes).unwrap(), heads);
    let mut over = heads.clone();
    over.push(heads[0].clone());
    assert!(super::encode_frontier(&over).is_err());
}

/// The gossip fast path (`governanceOp @74`): a signed op rides a
/// `ControlPayload::GovernanceOp` envelope byte for byte, and decodes
/// back to the same op.
#[test]
fn an_op_round_trips_through_the_gossip_arm() {
    use crate::capnp_envelope::{decode_community_envelope, encode_community_envelope};
    use crate::community::envelope::{CommunityEnvelope, ControlPayload};

    let original = op(GovernanceEntry::MemberLeft {}, vec![op_ref(3)]);
    let encoded = encode_op(&original).unwrap();
    let envelope = CommunityEnvelope::Control(ControlPayload::GovernanceOp {
        op: encoded.clone(),
    });
    let wire = encode_community_envelope(&envelope).unwrap();
    let CommunityEnvelope::Control(ControlPayload::GovernanceOp { op: carried }) =
        decode_community_envelope(&wire).unwrap()
    else {
        panic!("not the governanceOp arm");
    };
    assert_eq!(carried, encoded);
    assert_eq!(decode_op(&carried).unwrap().op, original);
}

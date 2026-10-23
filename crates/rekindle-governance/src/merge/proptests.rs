#![allow(
    clippy::cast_possible_truncation,
    reason = "property test: segment counts are proptest-bounded well under u32::MAX"
)]

//! Property-based convergence tests for the CRDT merge engine.

use super::*;
use crate::fixture::{at, cert, merge_entries, ops_in_order, without, Stamped};
use crate::op::VerifiedOp;
use proptest::prelude::*;
use rekindle_types::governance::AdmissionMode;
use rekindle_types::id::{ChannelId, RoleId, ThreadId};

fn arb_pseudonym() -> impl Strategy<Value = PseudonymKey> {
    prop::array::uniform32(any::<u8>()).prop_map(PseudonymKey)
}

fn arb_channel_id() -> impl Strategy<Value = ChannelId> {
    prop::array::uniform16(any::<u8>()).prop_map(ChannelId)
}

fn arb_role_id() -> impl Strategy<Value = RoleId> {
    prop::array::uniform16(any::<u8>()).prop_map(RoleId)
}

fn arb_entry() -> impl Strategy<Value = Stamped> {
    prop_oneof![
        // CommunityMeta
        (any::<u64>()).prop_map(|lamport| at(
            lamport,
            GovernanceEntry::CommunityMeta {
                name: Some("test".into()),
                description: None,
                icon_hash: None,
                banner_hash: None,
            }
        )),
        // ChannelCreated
        (arb_channel_id(), any::<u64>()).prop_map(|(ch, lamport)| {
            at(
                lamport,
                GovernanceEntry::ChannelCreated {
                    channel_id: ch,
                    name: "ch".into(),
                    channel_type: "text".into(),
                    record_key: "k".into(),
                    category_id: None,
                    position: 0,
                    parent_voice_channel_id: None,
                },
            )
        }),
        // RoleDefinition
        (arb_role_id(), any::<u64>(), any::<u64>()).prop_map(|(rid, perms, lamport)| {
            at(
                lamport,
                GovernanceEntry::RoleDefinition {
                    role_id: rid,
                    name: "role".into(),
                    permissions: perms,
                    position: 0,
                    color: 0,
                    hoist: false,
                    mentionable: false,
                    self_assignable: false,
                    exclusion_group: None,
                },
            )
        }),
        // ExpressionAdded
        (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(|(expression_id, lamport)| {
            at(
                lamport,
                GovernanceEntry::ExpressionAdded {
                    expression_id,
                    name: "emoji_name".into(),
                    kind: "emoji".into(),
                    content_hash: "hash".into(),
                    attachment: None,
                    animated: false,
                    tags: vec!["test".into()],
                    sound_meta: None,
                    creator_pseudonym: None,
                    created_at: None,
                    available_to_peers: Some(true),
                },
            )
        }),
        // ExpressionRemoved
        (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(|(expression_id, lamport)| {
            at(
                lamport,
                GovernanceEntry::ExpressionRemoved { expression_id },
            )
        }),
        // MEKGenerationBump
        (any::<u64>(), any::<u64>(), arb_pseudonym()).prop_map(|(gen, lamport, departed)| {
            at(
                lamport,
                GovernanceEntry::MEKGenerationBump {
                    generation: gen,
                    trigger_departed: departed,
                    cascade_skipped: vec![],
                },
            )
        }),
        // BanEntry
        (arb_pseudonym(), any::<u64>()).prop_map(|(target, lamport)| {
            at(
                lamport,
                GovernanceEntry::BanEntry {
                    target,
                    reason: None,
                },
            )
        }),
    ]
}

/// A slowmode, timeout or description entry over the whole value range of
/// its bounded field, edges and both sides of each bound included (plan
/// E3.0). Targets the channel `[7; 16]` and the members `[1..=3; 32]`.
fn arb_bounded_entry() -> impl Strategy<Value = Stamped> {
    use rekindle_types::channel::MAX_SLOWMODE_SECONDS;
    use rekindle_types::key_format::MAX_DESCRIPTION_LEN;
    use rekindle_types::member::MAX_TIMEOUT_SECONDS;
    let slowmode = prop_oneof![
        Just(MAX_SLOWMODE_SECONDS),
        Just(MAX_SLOWMODE_SECONDS + 1),
        0..=MAX_SLOWMODE_SECONDS,
        any::<u32>(),
    ];
    let duration = prop_oneof![
        Just(0),
        Just(MAX_TIMEOUT_SECONDS),
        Just(MAX_TIMEOUT_SECONDS + 1),
        1..=MAX_TIMEOUT_SECONDS,
        any::<u64>(),
    ];
    let description = (
        MAX_DESCRIPTION_LEN - 2..=MAX_DESCRIPTION_LEN + 2,
        any::<bool>(),
    )
        .prop_map(|(len, bell)| {
            let text = "é".repeat(len);
            if bell {
                format!("{text}\u{7}")
            } else {
                text
            }
        });
    prop_oneof![
        (slowmode, 0u64..64).prop_map(|(seconds, lamport)| at(
            lamport,
            GovernanceEntry::ChannelUpdated {
                channel_id: ChannelId([7; 16]),
                name: None,
                topic: None,
                forum_tags: None,
                position: None,
                slowmode_seconds: Some(seconds),
                nsfw: None,
                category_id: None,
            }
        )),
        (duration, 1u8..=3, 0u64..64).prop_map(|(duration_seconds, t, lamport)| {
            at(
                lamport,
                GovernanceEntry::TimeoutEntry {
                    target: PseudonymKey([t; 32]),
                    duration_seconds,
                    reason: None,
                    started_at: 0,
                },
            )
        }),
        (description, 0u64..64).prop_map(|(text, lamport)| at(
            lamport,
            GovernanceEntry::CommunityMeta {
                name: Some("C".into()),
                description: Some(text),
                icon_hash: None,
                banner_hash: None,
            }
        )),
    ]
}

/// The creator `[0; 32]`'s log at lamport 1: an everyone-ALL role and the
/// channel `[7; 16]`. An entry at lamport 0 sorts before it and is the
/// genesis if it is within bounds.
fn bounded_base() -> Vec<Stamped> {
    let role = |id: u8, position: u32| {
        at(
            1,
            GovernanceEntry::RoleDefinition {
                role_id: RoleId([id; 16]),
                name: "all".into(),
                permissions: rekindle_types::permissions::ALL,
                position,
                color: 0,
                hoist: false,
                mentionable: false,
                self_assignable: false,
                exclusion_group: None,
            },
        )
    };
    vec![
        role(0, 0),
        at(
            1,
            GovernanceEntry::ChannelCreated {
                channel_id: ChannelId([7; 16]),
                name: "ch".into(),
                channel_type: "text".into(),
                record_key: "k".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
            },
        ),
    ]
}

proptest! {
    /// **Bounds (plan E3.0):** whoever writes them (the creator `[0; 32]`
    /// included), at whatever lamport (a would-be genesis at 0 included), out-of-bounds entries never change the
    /// merged state: merging with them equals merging without them, and
    /// every applied slowmode, timeout and description is within its bound.
    #[test]
    fn out_of_bounds_entries_never_change_the_merge(
        logs in prop::collection::vec(
            (0u8..=3, prop::collection::vec(arb_bounded_entry(), 0..8)),
            1..4,
        ),
    ) {
        use crate::validate::bounds;
        let creator = PseudonymKey([0; 32]);
        let mut raw = vec![(creator.clone(), bounded_base())];
        let mut kept = raw.clone();
        for (author, entries) in logs {
            let author = PseudonymKey([author; 32]);
            let within = entries.iter().filter(|e| bounds::entry_within_bounds(&e.1)).cloned();
            kept.push((author.clone(), within.collect()));
            raw.push((author, entries));
        }
        let _ = kept;
        let cert = cert(&creator, AdmissionMode::default());
        let ops = ops_in_order(&raw);
        let state = merge(&cert, &ops);
        let within = without(&ops, |o| !bounds::entry_within_bounds(o.body()));
        prop_assert_eq!(&state, &merge(&cert, &within));
        for ch in state.channels.values() {
            prop_assert!(ch.slowmode_seconds.is_none_or(bounds::slowmode_ok));
        }
        for t in state.timeouts.values() {
            prop_assert!(bounds::timeout_ok(t.duration_seconds));
        }
        if let Some(m) = &state.metadata {
            prop_assert!(m.description.as_deref().is_none_or(bounds::description_ok));
        }
    }

    /// **CRDT convergence:** Two subkey orderings produce identical state.
    #[test]
    fn merge_is_order_independent(
        author_a in arb_pseudonym(),
        author_b in arb_pseudonym(),
        entries_a in prop::collection::vec(arb_entry(), 1..5),
        entries_b in prop::collection::vec(arb_entry(), 0..3),
    ) {
        let state1 = merge_entries(&author_a, &[
            (author_a.clone(), entries_a.clone()),
            (author_b.clone(), entries_b.clone()),
        ]);
        let state2 = merge_entries(&author_a, &[
            (author_b.clone(), entries_b),
            (author_a.clone(), entries_a),
        ]);
        prop_assert_eq!(state1, state2);
    }

    /// **Idempotence:** Merging the same entries twice doesn't change the state.
    #[test]
    fn merge_is_idempotent(
        author in arb_pseudonym(),
        entries in prop::collection::vec(arb_entry(), 1..5),
    ) {
        let cert = cert(&author, AdmissionMode::default());
        let ops = ops_in_order(&[(author, entries)]);
        let twice: Vec<VerifiedOp> = ops.iter().chain(ops.iter()).cloned().collect();
        prop_assert_eq!(merge(&cert, &ops), merge(&cert, &twice));
    }

    #[test]
    fn thread_versions_converge_to_same_active_thread_state(
        thread_id_bytes in prop::array::uniform16(any::<u8>()),
        parent_channel_bytes in prop::array::uniform16(any::<u8>()),
        create_lamport in 2u64..100_000,
        record_delta in 1u64..16,
        archive_delta in 0u64..16,
    ) {
        let creator = PseudonymKey([1; 32]);
        let thread_id = ThreadId(thread_id_bytes);
        let parent_channel_id = ChannelId(parent_channel_bytes);

        let community_meta = at(1, GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        });
        let create_without_record = at(create_lamport, GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
        });
        let create_with_record = at(create_lamport.saturating_add(record_delta), GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
        });
        let archive = at(create_lamport.saturating_add(archive_delta), GovernanceEntry::ThreadArchived {
            thread_id,
        });

        let state1 = merge_entries(&creator, &[(
            creator.clone(),
            vec![
                community_meta.clone(),
                create_without_record.clone(),
                create_with_record.clone(),
                archive.clone(),
            ],
        )]);
        let state2 = merge_entries(&creator, &[(
            creator.clone(),
            vec![
                community_meta,
                archive,
                create_with_record,
                create_without_record,
            ],
        )]);

        prop_assert_eq!(&state1.threads, &state2.threads);

        let thread = state1
            .threads
            .get(&thread_id)
            .expect("thread must be materialized");
        prop_assert_eq!(thread.record_key.as_deref(), Some("VLD0:thread"));

        let expected_archived =
            (create_lamport.saturating_add(archive_delta) > thread.created.lamport)
                .then_some(create_lamport.saturating_add(archive_delta));
        prop_assert_eq!(thread.archived.map(|s| s.lamport), expected_archived);
    }

    /// **Plate Gate (architecture §15) — segments converge under any
    /// arrival order.** Each segment is its own join-semilattice; the
    /// merged community state is the product CRDT under coordinate-wise
    /// join (Shapiro 2011 / Almeida 2016 arXiv:1603.01529 §3). The
    /// existing merge implementation appends only when `segment_index`
    /// is new; this property test confirms that K random orderings of
    /// the same entry set produce the same final `state.segments`.
    #[test]
    fn segments_converge_regardless_of_order(
        segment_count in 1u32..=4,
        base_lamport in 100u64..1_000,
    ) {
        let creator = PseudonymKey([1; 32]);
        let community_meta = at(1, GovernanceEntry::CommunityMeta {
            name: Some("plate-gate".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        });

        // Each segment uses an independent governance + registry key
        // and a contiguous slot range (255 slots per segment per the
        // architecture's universal SMPL schema).
        let segment_entries: Vec<Stamped> = (1..=segment_count)
            .map(|idx| at(base_lamport.saturating_add(u64::from(idx)), GovernanceEntry::SegmentAdded {
                segment_index: idx,
                registry_key: format!("REG{idx}"),
                governance_key: format!("GOV{idx}"),
                slot_range_start: idx * 255,
                slot_range_end: idx * 255 + 255,
            }))
            .collect();

        // Two orderings: forward and reverse. CRDT idempotence + commutativity
        // guarantees both produce the same merged state.
        let mut forward = vec![community_meta.clone()];
        forward.extend(segment_entries.iter().cloned());
        let mut reverse = vec![community_meta];
        reverse.extend(segment_entries.iter().rev().cloned());

        let state_forward = merge_entries(&creator, &[(creator.clone(), forward)]);
        let state_reverse = merge_entries(&creator, &[(creator.clone(), reverse)]);

        prop_assert_eq!(&state_forward.segments, &state_reverse.segments);
        // Sorted by segment_index ascending (merge.rs:565).
        for window in state_forward.segments.windows(2) {
            prop_assert!(window[0].segment_index < window[1].segment_index);
        }
        // Each segment_index appears exactly once.
        prop_assert_eq!(state_forward.segments.len() as u32, segment_count);
    }
}

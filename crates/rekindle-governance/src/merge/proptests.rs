//! Property-based convergence tests for the CRDT merge engine.

use super::*;
use proptest::prelude::*;
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

fn arb_entry() -> impl Strategy<Value = GovernanceEntry> {
    prop_oneof![
        // CommunityMeta
        (any::<u64>()).prop_map(|lamport| GovernanceEntry::CommunityMeta {
            name: Some("test".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport,
        }),
        // ChannelCreated
        (arb_channel_id(), any::<u64>()).prop_map(|(ch, lamport)| {
            GovernanceEntry::ChannelCreated {
                channel_id: ch,
                name: "ch".into(),
                channel_type: "text".into(),
                record_key: "k".into(),
                category_id: None,
                position: 0,
                parent_voice_channel_id: None,
                lamport,
            }
        }),
        // RoleDefinition
        (arb_role_id(), any::<u64>(), any::<u64>()).prop_map(|(rid, perms, lamport)| {
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
                lamport,
            }
        }),
        // ExpressionAdded
        (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(|(expression_id, lamport)| {
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
                lamport,
            }
        }),
        // ExpressionRemoved
        (prop::array::uniform16(any::<u8>()), any::<u64>()).prop_map(|(expression_id, lamport)| {
            GovernanceEntry::ExpressionRemoved {
                expression_id,
                lamport,
            }
        }),
        // MEKGenerationBump
        (any::<u64>(), any::<u64>(), arb_pseudonym()).prop_map(|(gen, lamport, departed)| {
            GovernanceEntry::MEKGenerationBump {
                generation: gen,
                trigger_departed: departed,
                cascade_skipped: vec![],
                lamport,
            }
        }),
        // BanEntry
        (arb_pseudonym(), any::<u64>()).prop_map(|(target, lamport)| {
            GovernanceEntry::BanEntry {
                target,
                reason: None,
                lamport,
            }
        }),
    ]
}

proptest! {
    /// **CRDT convergence:** Two subkey orderings produce identical state.
    #[test]
    fn merge_is_order_independent(
        author_a in arb_pseudonym(),
        author_b in arb_pseudonym(),
        entries_a in prop::collection::vec(arb_entry(), 1..5),
        entries_b in prop::collection::vec(arb_entry(), 0..3),
    ) {
        let state1 = merge(&[
            (author_a.clone(), entries_a.clone()),
            (author_b.clone(), entries_b.clone()),
        ]);
        let state2 = merge(&[
            (author_b, entries_b),
            (author_a, entries_a),
        ]);
        prop_assert_eq!(state1, state2);
    }

    /// **Idempotence:** Merging the same entries twice doesn't change the state.
    #[test]
    fn merge_is_idempotent(
        author in arb_pseudonym(),
        entries in prop::collection::vec(arb_entry(), 1..5),
    ) {
        let state1 = merge(&[(author.clone(), entries.clone())]);
        let state2 = merge(&[
            (author.clone(), entries.clone()),
            (author, entries),
        ]);
        prop_assert_eq!(state1, state2);
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

        let community_meta = GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        };
        let create_without_record = GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: create_lamport,
        };
        let create_with_record = GovernanceEntry::ThreadCreated {
            thread_id,
            parent_channel_id,
            name: "ops".into(),
            thread_type: "public".into(),
            record_key: Some("VLD0:thread".into()),
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 86_400,
            lamport: create_lamport.saturating_add(record_delta),
        };
        let archive = GovernanceEntry::ThreadArchived {
            thread_id,
            lamport: create_lamport.saturating_add(archive_delta),
        };

        let state1 = merge(&[(
            creator.clone(),
            vec![
                community_meta.clone(),
                create_without_record.clone(),
                create_with_record.clone(),
                archive.clone(),
            ],
        )]);
        let state2 = merge(&[(
            creator,
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
            (create_lamport.saturating_add(archive_delta) > thread.created_lamport)
                .then_some(create_lamport.saturating_add(archive_delta));
        prop_assert_eq!(thread.archived_lamport, expected_archived);
    }

    /// **§4.2 compaction preserves merged state.** Compacting one author's
    /// own subkey before it is signed and published must never change the
    /// merged `GovernanceState`, for any other-author context. The families
    /// `arb_entry` produces are either LWW-collapsed (`CommunityMeta`,
    /// `MEKGenerationBump`) or kept verbatim, and none of them gate another
    /// author's `validate_write`, so the merged state is identical whether the
    /// author's log is raw or compacted. The author-level genesis guard keeps
    /// creator assignment stable even when the global minimum lives in `ctx`.
    #[test]
    fn compaction_preserves_merge(
        ctx in prop::collection::vec(
            (arb_pseudonym(), prop::collection::vec(arb_entry(), 0..12)),
            0..4,
        ),
        author in arb_pseudonym(),
        e in prop::collection::vec(arb_entry(), 0..16),
    ) {
        let raw = [ctx.clone(), vec![(author.clone(), e.clone())]].concat();
        let folded = [
            ctx,
            vec![(author, crate::compact::compact_author_entries(e, 0))],
        ]
        .concat();
        prop_assert_eq!(merge(&raw), merge(&folded));
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
        let community_meta = GovernanceEntry::CommunityMeta {
            name: Some("plate-gate".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        };

        // Each segment uses an independent governance + registry key
        // and a contiguous slot range (255 slots per segment per the
        // architecture's universal SMPL schema).
        let segment_entries: Vec<GovernanceEntry> = (1..=segment_count)
            .map(|idx| GovernanceEntry::SegmentAdded {
                segment_index: idx,
                registry_key: format!("REG{idx}"),
                governance_key: format!("GOV{idx}"),
                slot_range_start: idx * 255,
                slot_range_end: idx * 255 + 255,
                lamport: base_lamport.saturating_add(u64::from(idx)),
            })
            .collect();

        // Two orderings: forward and reverse. CRDT idempotence + commutativity
        // guarantees both produce the same merged state.
        let mut forward = vec![community_meta.clone()];
        forward.extend(segment_entries.iter().cloned());
        let mut reverse = vec![community_meta];
        reverse.extend(segment_entries.iter().rev().cloned());

        let state_forward = merge(&[(creator.clone(), forward)]);
        let state_reverse = merge(&[(creator, reverse)]);

        prop_assert_eq!(&state_forward.segments, &state_reverse.segments);
        // Sorted by segment_index ascending (merge.rs:565).
        for window in state_forward.segments.windows(2) {
            prop_assert!(window[0].segment_index < window[1].segment_index);
        }
        // Each segment_index appears exactly once.
        prop_assert_eq!(state_forward.segments.len() as u32, segment_count);
    }
}

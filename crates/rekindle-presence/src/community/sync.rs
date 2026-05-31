//! Phase 21 REDO — initial-sync handshake orchestrator.
//!
//! Pre-port lived in `src-tauri/services/community/presence/sync.rs`.
//! On first peer-attach: broadcast `PresenceUpdate` to mesh, fire one
//! `SyncRequest` per channel, then catch up missing messages by
//! reading each channel's SMPL log record directly. Pure
//! orchestration parameterised over [`CommunityPresenceDeps`].

use std::sync::Arc;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

use crate::deps::CommunityPresenceDeps;

/// On first successful peer-attachment, broadcast our presence row +
/// a per-channel `SyncRequest` + catch up missing messages by reading
/// the SMPL channel-log records directly.
///
/// `d` is the gossip fan-out degree decided by `presence_poll_tick`
/// — when `d == 0` we skip the PresenceUpdate broadcast (no online
/// peers anyway) but still run the channel-log catch-up read since
/// SMPL records are accessible without peers.
pub async fn run_initial_sync<D: CommunityPresenceDeps>(
    deps: Arc<D>,
    community_id: &str,
    d: usize,
) {
    if d > 0 {
        let my_pk = deps.my_pseudonym_for_community(community_id);
        let our_route = deps.our_route_blob();
        if our_route.is_some() {
            let envelope = CommunityEnvelope::PresenceUpdate {
                pseudonym_key: my_pk,
                status: deps.current_presence_status_str(community_id),
                game_info: None,
                route_blob: our_route,
            };
            deps.send_to_mesh(community_id, envelope);
        } else {
            tracing::warn!(
                community = %community_id,
                "skipping PresenceUpdate broadcast — route_blob not yet available",
            );
            return;
        }
    }

    let channel_ids = deps.channel_ids_for_community(community_id);
    for channel_id in &channel_ids {
        let last_ts = deps
            .last_channel_message_timestamp(community_id, channel_id)
            .await;
        let sync_req = CommunityEnvelope::Control(ControlPayload::SyncRequest {
            channel_id: channel_id.clone(),
            since_timestamp: last_ts.cast_unsigned(),
        });
        deps.send_to_mesh(community_id, sync_req);
        deps.mark_pending_sync(community_id, channel_id, 1);
    }

    let channel_entries = deps.channel_log_keys_for_community(community_id);
    let member_count = deps.member_count_for_community(community_id);

    if !channel_entries.is_empty() && member_count > 0 {
        for (channel_id, record_key) in &channel_entries {
            match deps
                .read_all_channel_messages(record_key, member_count)
                .await
            {
                Ok(messages) if !messages.is_empty() => {
                    tracing::debug!(
                        community = %community_id,
                        channel = %channel_id,
                        count = messages.len(),
                        "caught up from SMPL channel record",
                    );
                    deps.persist_channel_catchup(community_id, channel_id, messages);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::debug!(
                        community = %community_id,
                        channel = %channel_id,
                        %error,
                        "SMPL channel catch-up failed",
                    );
                }
            }
        }
    }

    deps.mark_initial_sync_done(community_id);
    tracing::info!(community = %community_id, "initial sync complete");
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use parking_lot::Mutex;
    use rekindle_protocol::dht::community::channel_record::ChannelMessage;

    use super::*;
    use crate::community::test_fixture::{MockCommunityDeps, MockState};

    #[tokio::test]
    async fn no_peers_skips_presence_broadcast_but_still_marks_done() {
        let deps = Arc::new(MockCommunityDeps {
            state: Mutex::new(MockState {
                channels: vec!["ch1".into()],
                ..Default::default()
            }),
        });
        run_initial_sync(Arc::clone(&deps), "c1", 0).await;
        let st = deps.state.lock();
        assert_eq!(st.sent_envelopes.len(), 1);
        assert!(matches!(
            st.sent_envelopes[0].1,
            CommunityEnvelope::Control(ControlPayload::SyncRequest { .. })
        ));
        assert_eq!(st.pending_syncs, vec![("c1".into(), "ch1".into(), 1)]);
        assert_eq!(st.initial_done, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn no_route_blob_aborts_early() {
        let deps = Arc::new(MockCommunityDeps {
            state: Mutex::new(MockState {
                my_pk: "me".into(),
                our_route: None,
                channels: vec!["ch1".into()],
                ..Default::default()
            }),
        });
        run_initial_sync(Arc::clone(&deps), "c1", 3).await;
        let st = deps.state.lock();
        assert!(st.sent_envelopes.is_empty());
        assert!(st.pending_syncs.is_empty());
        assert!(st.initial_done.is_empty());
    }

    #[tokio::test]
    async fn full_path_sends_presence_then_per_channel_sync_then_catchup() {
        let mut read = HashMap::new();
        read.insert(
            "rec1".to_string(),
            Ok(vec![ChannelMessage {
                sequence: 1,
                sender_pseudonym: "alice".into(),
                ciphertext: vec![0u8; 16],
                mek_generation: 0,
                timestamp: 1000,
                reply_to: None,
                lamport_ts: 1,
                message_id: Some("m1".into()),
                attachment: None,
                flags: 0,
                mentioned_pseudonyms: Vec::new(),
                mentioned_roles: Vec::new(),
            }]),
        );
        let deps = Arc::new(MockCommunityDeps {
            state: Mutex::new(MockState {
                my_pk: "me".into(),
                our_route: Some(vec![1, 2, 3]),
                status: "online".into(),
                channels: vec!["ch1".into(), "ch2".into()],
                channel_logs: vec![("ch1".into(), "rec1".into())],
                member_count: 5,
                read_results: read,
                ..Default::default()
            }),
        });
        run_initial_sync(Arc::clone(&deps), "c1", 3).await;
        let st = deps.state.lock();
        assert_eq!(st.sent_envelopes.len(), 3);
        assert!(matches!(
            st.sent_envelopes[0].1,
            CommunityEnvelope::PresenceUpdate { .. }
        ));
        assert_eq!(st.pending_syncs.len(), 2);
        assert_eq!(st.catchups, vec![("c1".into(), "ch1".into(), 1)]);
        assert_eq!(st.initial_done, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn empty_channel_log_skips_catchup_read() {
        let deps = Arc::new(MockCommunityDeps {
            state: Mutex::new(MockState {
                my_pk: "me".into(),
                our_route: Some(vec![1]),
                status: "online".into(),
                channels: vec!["ch1".into()],
                channel_logs: Vec::new(),
                member_count: 5,
                ..Default::default()
            }),
        });
        run_initial_sync(Arc::clone(&deps), "c1", 3).await;
        let st = deps.state.lock();
        assert_eq!(st.sent_envelopes.len(), 2);
        assert!(st.catchups.is_empty());
        assert_eq!(st.initial_done, vec!["c1".to_string()]);
    }
}

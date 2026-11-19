//! Phase 21 REDO — initial-sync handshake orchestrator.
//!
//! Pre-port lived in `src-tauri/services/community/presence/sync.rs`.
//! On first peer-attach: broadcast `PresenceUpdate` to mesh. Channel
//! history is each author's outbox (`M`/4), caught up by the Host on
//! load, join and every key install (plan E3.3), never here. Pure
//! orchestration parameterised over [`CommunityPresenceDeps`].

use std::sync::Arc;

use rekindle_codec::community::envelope::CommunityEnvelope;

use crate::deps::CommunityPresenceDeps;

/// On first successful peer-attachment, broadcast our presence row.
///
/// `d` is the gossip fan-out degree decided by `presence_poll_tick`
/// — when `d == 0` we skip the PresenceUpdate broadcast (no online
/// peers anyway) and still mark the sync done.
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

    deps.mark_initial_sync_done(community_id);
    tracing::info!(community = %community_id, "initial sync complete");
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::community::test_fixture::{MockCommunityDeps, MockState};

    #[tokio::test]
    async fn no_peers_skips_presence_broadcast_but_still_marks_done() {
        let deps = Arc::new(MockCommunityDeps::new(MockState {
            ..Default::default()
        }));
        run_initial_sync(Arc::clone(&deps), "c1", 0).await;
        let st = deps.state.lock();
        assert!(st.sent_envelopes.is_empty());
        assert_eq!(st.initial_done, vec!["c1".to_string()]);
    }

    #[tokio::test]
    async fn no_route_blob_aborts_early() {
        let deps = Arc::new(MockCommunityDeps::new(MockState {
            my_pk: "me".into(),
            our_route: None,
            ..Default::default()
        }));
        run_initial_sync(Arc::clone(&deps), "c1", 3).await;
        let st = deps.state.lock();
        assert!(st.sent_envelopes.is_empty());
        assert!(st.initial_done.is_empty());
    }

    #[tokio::test]
    async fn full_path_sends_presence_and_marks_done() {
        let deps = Arc::new(MockCommunityDeps::new(MockState {
            my_pk: "me".into(),
            our_route: Some(vec![1, 2, 3]),
            status: "online".into(),
            ..Default::default()
        }));
        run_initial_sync(Arc::clone(&deps), "c1", 3).await;
        let st = deps.state.lock();
        assert_eq!(st.sent_envelopes.len(), 1);
        assert!(matches!(
            st.sent_envelopes[0].1,
            CommunityEnvelope::PresenceUpdate { .. }
        ));
        assert_eq!(st.initial_done, vec!["c1".to_string()]);
    }
}

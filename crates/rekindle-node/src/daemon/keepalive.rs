//! The daemon's record keepalive (plan C7.8b): the shared loop
//! (`rekindle_protocol::dht::pool::keepalive::run`) over the records the
//! community holds, on the community's scope, so leaving or locking stops
//! it. The Tauri host runs the same loop over its record inventory.

use std::sync::Arc;

use super::dispatch::DaemonContext;

/// Start the community's keepalive, once per community per unlock.
pub(crate) fn start(ctx: &DaemonContext, community_id: &str) {
    if !ctx.community_runtime.start_keepalive_once(community_id) {
        return;
    }
    let Some(node) = ctx.transport.read().clone() else {
        return;
    };
    let runtime = Arc::clone(&ctx.community_runtime);
    let id = community_id.to_string();
    ctx.community_scope(community_id).spawn_with_token_or_drop(
        "community DHT keepalive",
        move |stop| async move {
            let pool_node = Arc::clone(&node);
            rekindle_protocol::dht::pool::keepalive::run(
                move || pool_node.records(),
                move || {
                    Some(
                        runtime
                            .held_leases(&id)
                            .into_iter()
                            .filter_map(|lease| {
                                rekindle_transport::broadcast::dht_writes::key_of(&node, lease)
                            })
                            .collect(),
                    )
                },
                stop,
            )
            .await;
        },
    );
}

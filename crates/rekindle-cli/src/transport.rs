//! Transport node lifecycle management for the CLI.
//!
//! Provides `TransportHandle` which owns or borrows a running `TransportNode`,
//! and `CliHandler` which implements the `InboundHandler` trait to bridge
//! transport events into the CLI/TUI event system.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use tokio::sync::mpsc;
use tracing::info;

use rekindle_transport::{
    InboundHandler, TransportEvent, TransportNode,
    VerifiedSender,
    crypto::mek::MekCache,
    payload::dm::DmPayload,
    payload::gossip::{GossipPayload, SignedGossipEnvelope},
    payload::rpc::{CallResponse, InboundCall},
    payload::voice::VoicePayload,
};

use crate::config::schema::Config;
use crate::cli::Cli;

// ── CLI Event (transport → CLI/TUI) ─────────────────────────────────────

/// Events from the transport layer, bridged to the CLI/TUI event loop.
///
/// The `InboundHandler` pushes these into an mpsc channel. The CLI's
/// one-shot commands drain them after each operation. The TUI's event
/// loop drains them continuously in `tokio::select!`.
#[derive(Debug, Clone)]
pub enum CliEvent {
    Dm {
        sender_key: String,
        sender_name: String,
        payload: DmPayload,
        timestamp: u64,
    },
    Gossip {
        community_id: String,
        sender_pseudonym: String,
        payload: GossipPayload,
        lamport_ts: u64,
    },
    ValueChange {
        record_key: String,
        subkeys: Vec<u32>,
    },
    Transport(TransportEvent),
    VoicePacket {
        sender_key: String,
    },
}

// ── Transport Handle ────────────────────────────────────────────────────

/// Owns a running `TransportNode` and provides the shared state
/// (MEK cache, event channel) that command modules need.
///
/// The event channel (`event_rx`) receives authenticated payloads from
/// the `CliHandler`'s `InboundHandler` implementation. Watch commands
/// drain it for live-streaming. One-shot commands ignore it.
pub struct TransportHandle {
    node: TransportNode,
    pub mek_cache: Arc<RwLock<MekCache>>,
    pub event_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<CliEvent>>>,
}

impl TransportHandle {
    /// Access the underlying transport node.
    pub fn node(&self) -> &TransportNode {
        &self.node
    }

    /// Shut down the transport node gracefully.
    pub async fn shutdown_if_owned(self) -> anyhow::Result<()> {
        info!("shutting down transport node");
        self.node
            .shutdown()
            .await
            .map_err(|e| anyhow::anyhow!("transport shutdown: {e}"))
    }
}

// ── CLI Handler (InboundHandler impl) ───────────────────────────────────

/// Implements `InboundHandler` to bridge transport events to the CLI.
///
/// Every callback pushes a `CliEvent` into the mpsc channel for consumption
/// by watch commands and the TUI event loop. Additionally, when a
/// `FriendRequest` DM arrives, the handler persists it to the session file
/// so `rekindle friend requests` and `rekindle friend accept` can access it
/// across CLI invocations.
struct CliHandler {
    event_tx: mpsc::UnboundedSender<CliEvent>,
    /// Path to the session JSON file for persisting pending friend requests.
    session_path: std::path::PathBuf,
}

#[allow(clippy::manual_async_fn)]
impl InboundHandler for CliHandler {
    fn on_dm(
        &self,
        sender: &VerifiedSender,
        payload: DmPayload,
        timestamp: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        let tx = self.event_tx.clone();
        let sender_key = sender.public_key.clone();
        let sender_name = sender.display_name.clone();
        let session_path = self.session_path.clone();
        let payload_clone = payload.clone();
        async move {
            // Persist friend requests to the session file so they survive
            // across CLI invocations. The user can then run
            // `rekindle friend accept` in a separate command.
            if let DmPayload::FriendRequest {
                ref display_name,
                ref message,
                ref prekey_bundle,
                ref profile_dht_key,
                ref route_blob,
                ref mailbox_dht_key,
                ref invite_id,
            } = payload_clone
            {
                let pending = rekindle_transport::PendingFriendRequest {
                    public_key: sender_key.clone(),
                    display_name: display_name.clone(),
                    message: message.clone(),
                    profile_dht_key: profile_dht_key.clone(),
                    route_blob: route_blob.clone(),
                    mailbox_dht_key: mailbox_dht_key.clone(),
                    prekey_bundle: prekey_bundle.clone(),
                    invite_id: invite_id.clone(),
                    received_at: timestamp,
                };

                // Load → mutate → save atomically
                if let Ok(Some(mut session)) =
                    rekindle_transport::Session::load(&session_path)
                {
                    session.add_pending_friend_request(pending);
                    if let Err(e) = session.save(&session_path) {
                        tracing::warn!(error = %e, "failed to persist pending friend request");
                    } else {
                        tracing::info!(
                            from = %sender_key,
                            name = %display_name,
                            "friend request received and persisted"
                        );
                    }
                }
            }

            let _ = tx.send(CliEvent::Dm {
                sender_key,
                sender_name,
                payload: payload_clone,
                timestamp,
            });
        }
    }

    fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: GossipPayload,
        lamport_ts: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        let tx = self.event_tx.clone();
        let cid = community_id.to_owned();
        let sender = sender_pseudonym.to_owned();
        async move {
            let _ = tx.send(CliEvent::Gossip {
                community_id: cid,
                sender_pseudonym: sender,
                payload,
                lamport_ts,
            });
        }
    }

    fn on_gossip_forward(
        &self,
        _envelope: &SignedGossipEnvelope,
    ) -> impl std::future::Future<Output = ()> + Send {
        // CLI doesn't forward gossip — that's the transport's job
        async {}
    }

    fn on_voice(
        &self,
        sender_key: &str,
        _packet: VoicePayload,
    ) -> impl std::future::Future<Output = ()> + Send {
        let tx = self.event_tx.clone();
        let key = sender_key.to_owned();
        async move {
            let _ = tx.send(CliEvent::VoicePacket { sender_key: key });
        }
    }

    fn on_call(
        &self,
        _sender_pseudonym: Option<&str>,
        _request: InboundCall,
    ) -> impl std::future::Future<Output = CallResponse> + Send {
        // CLI acknowledges RPC calls but doesn't serve data
        // (it's a client, not a community archiver)
        async move { CallResponse::Ack }
    }

    fn on_value_change(
        &self,
        record_key: &str,
        changed_subkeys: Vec<u32>,
        _first_value: Option<Vec<u8>>,
    ) -> impl std::future::Future<Output = ()> + Send {
        let tx = self.event_tx.clone();
        let key = record_key.to_owned();
        async move {
            let _ = tx.send(CliEvent::ValueChange {
                record_key: key,
                subkeys: changed_subkeys,
            });
        }
    }

    fn on_event(
        &self,
        event: TransportEvent,
    ) -> impl std::future::Future<Output = ()> + Send {
        let tx = self.event_tx.clone();
        async move {
            let _ = tx.send(CliEvent::Transport(event));
        }
    }
}

// ── Acquire ─────────────────────────────────────────────────────────────

/// Start a transport node and return a handle.
///
/// Creates the `CliHandler`, starts the node, waits for network attachment,
/// and returns the handle with shared state.
pub async fn acquire(cfg: &Config, cli: &Cli) -> anyhow::Result<TransportHandle> {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let session_path = crate::helpers::session_path()?;
    let handler = Arc::new(CliHandler {
        event_tx: event_tx.clone(),
        session_path,
    });

    let transport_config = cfg.to_transport_config(cli)?;

    info!(
        namespace = %transport_config.namespace,
        storage = %transport_config.storage_dir,
        "starting transport node"
    );

    let node = TransportNode::start(transport_config, handler)
        .await
        .map_err(|e| anyhow::anyhow!("failed to start transport node: {e}"))?;

    // Wait for network attachment
    let timeout_secs = 30u64; // Default attach timeout
    wait_for_attachment(&node, timeout_secs).await?;

    let mek_cache = Arc::new(RwLock::new(MekCache::new()));

    // event_tx is owned solely by the CliHandler — no redundant copy stored.
    // The handler pushes events; watch commands drain event_rx.
    // M2 TUI bridge will restructure acquire() to return the sender separately.
    drop(event_tx);

    Ok(TransportHandle {
        node,
        mek_cache,
        event_rx: Arc::new(tokio::sync::Mutex::new(event_rx)),
    })
}

/// Wait for the transport node to attach to the network.
///
/// Polls the shared state until attachment is confirmed or timeout expires.
/// Attachment is confirmed when `is_attached()` returns true AND
/// `public_internet_ready()` returns true.
async fn wait_for_attachment(node: &TransportNode, timeout_secs: u64) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let poll_interval = Duration::from_millis(250);

    loop {
        if node.is_ready() {
            info!("network attached and public internet ready");
            return Ok(());
        }

        if tokio::time::Instant::now() > deadline {
            let state = node.shared().attachment_state();
            return Err(crate::error::CliError::Timeout(format!(
                "network attachment timed out after {timeout_secs}s (state: {state})"
            )).into());
        }

        tokio::time::sleep(poll_interval).await;
    }
}

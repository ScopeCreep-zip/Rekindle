//! DeliveryEngine — commoditized message delivery over the Veilid network.
//!
//! The chat layer calls `deliver()` or `deliver_community()` with a durability
//! preference. The engine handles route resolution, direct send, gossip broadcast,
//! fallback behavior, and circuit breaker integration internally.
//!
//! Durability tiers:
//!
//! - `Durable`: DHT write has already been performed by the caller (channel log,
//!   inbox entry, registry update). The engine attempts a best-effort direct send
//!   or gossip broadcast as a latency optimization. Failure is silent — the DHT
//!   write is the guaranteed delivery path.
//!
//! - `Ephemeral`: No DHT write. The engine attempts direct send or gossip broadcast.
//!   If the peer is unreachable or the mesh is empty, the message is dropped.
//!   Used for typing indicators, presence pings, and other loss-tolerant signals.

use std::sync::Arc;

use tracing::{debug, warn};

use rekindle_types::transport::{Durability, DeliveryReport};

use crate::broadcast::BroadcastManager;
use crate::broadcast::send::Sender;
use crate::config::TransportConfig;
use crate::resolver::{RouteResolver, ResolveResult};

pub struct DeliveryEngine {
    resolver: Arc<RouteResolver>,
    broadcast: Arc<BroadcastManager>,
    api: veilid_core::VeilidAPI,
    config: Arc<TransportConfig>,
}

impl DeliveryEngine {
    pub fn new(
        resolver: Arc<RouteResolver>,
        broadcast: Arc<BroadcastManager>,
        api: veilid_core::VeilidAPI,
        config: Arc<TransportConfig>,
    ) -> Self {
        Self { resolver, broadcast, api, config }
    }

    pub fn resolver(&self) -> &Arc<RouteResolver> {
        &self.resolver
    }

    /// Deliver a message to a specific peer.
    ///
    /// For `Durable`: the caller has already written to DHT. This attempts
    /// a direct app_message as a latency optimization. Send failure is silent.
    ///
    /// For `Ephemeral`: this is the only delivery path. Send failure is returned.
    pub async fn deliver(
        &self,
        peer_key: &str,
        data: &[u8],
        durability: Durability,
    ) -> DeliveryReport {
        match self.resolver.resolve(peer_key).await {
            ResolveResult::Found(target) => {
                let sender = Sender::new(self.api.clone(), Arc::clone(&self.config));
                match sender.send_raw(&target, data).await {
                    Ok(()) => {
                        self.resolver.record_success(peer_key);
                        DeliveryReport { sent: true, peers_reached: 1, error: None }
                    }
                    Err(e) => {
                        self.resolver.record_failure(peer_key);
                        let err_msg = format!("send failed: {e}");
                        match durability {
                            Durability::Durable => {
                                debug!(
                                    peer = &peer_key[..16.min(peer_key.len())],
                                    error = %e,
                                    "durable delivery: direct send failed — DHT covers it"
                                );
                                DeliveryReport { sent: false, peers_reached: 0, error: None }
                            }
                            Durability::Ephemeral => {
                                warn!(
                                    peer = &peer_key[..16.min(peer_key.len())],
                                    error = %e,
                                    "ephemeral delivery failed — message dropped"
                                );
                                DeliveryReport { sent: false, peers_reached: 0, error: Some(err_msg) }
                            }
                        }
                    }
                }
            }
            ResolveResult::UnknownPeer => {
                let msg = "unknown peer — no profile_dht_key registered";
                match durability {
                    Durability::Durable => {
                        debug!(peer = &peer_key[..16.min(peer_key.len())], msg);
                        DeliveryReport { sent: false, peers_reached: 0, error: None }
                    }
                    Durability::Ephemeral => {
                        DeliveryReport { sent: false, peers_reached: 0, error: Some(msg.into()) }
                    }
                }
            }
            ResolveResult::NoRoute => {
                let msg = "peer has no published route";
                match durability {
                    Durability::Durable => {
                        debug!(peer = &peer_key[..16.min(peer_key.len())], msg);
                        DeliveryReport { sent: false, peers_reached: 0, error: None }
                    }
                    Durability::Ephemeral => {
                        DeliveryReport { sent: false, peers_reached: 0, error: Some(msg.into()) }
                    }
                }
            }
            ResolveResult::CircuitOpen => {
                let msg = "circuit breaker open — peer has too many recent failures";
                debug!(peer = &peer_key[..16.min(peer_key.len())], msg);
                match durability {
                    Durability::Durable => DeliveryReport { sent: false, peers_reached: 0, error: None },
                    Durability::Ephemeral => DeliveryReport { sent: false, peers_reached: 0, error: Some(msg.into()) },
                }
            }
        }
    }

    /// Deliver a message to all peers in a community's gossip mesh.
    ///
    /// For `Durable`: the caller has already written to DHT (channel log,
    /// governance manifest, etc). The gossip broadcast is a latency optimization
    /// that tells online peers "read the DHT now" instead of waiting for their
    /// next poll cycle.
    ///
    /// For `Ephemeral`: the gossip broadcast is the only delivery path (typing
    /// indicators, presence). No DHT write exists. If the mesh is empty,
    /// the message is silently dropped.
    pub async fn deliver_community(
        &self,
        community_id: &str,
        data: &[u8],
        durability: Durability,
    ) -> DeliveryReport {
        let report = self.broadcast.broadcast_to_mesh(community_id, data).await;
        let peers_reached = u32::try_from(report.delivered).unwrap_or(u32::MAX);
        let peers_failed = report.failures.len();

        if peers_reached > 0 {
            debug!(
                community = &community_id[..20.min(community_id.len())],
                delivered = peers_reached,
                failed = peers_failed,
                "community delivery complete"
            );
        } else if !report.failures.is_empty() {
            for (peer, err) in &report.failures {
                debug!(peer = &peer[..16.min(peer.len())], error = %err, "mesh peer send failed");
            }
        }

        match durability {
            Durability::Durable => {
                // Broadcast failures are acceptable — DHT covers delivery
                if peers_reached == 0 && peers_failed == 0 {
                    debug!(
                        community = &community_id[..20.min(community_id.len())],
                        "durable community delivery: mesh empty — DHT covers it"
                    );
                }
                DeliveryReport { sent: peers_reached > 0, peers_reached, error: None }
            }
            Durability::Ephemeral => {
                if peers_reached == 0 && peers_failed == 0 {
                    // Empty mesh — ephemeral message silently dropped
                    DeliveryReport { sent: false, peers_reached: 0, error: None }
                } else {
                    DeliveryReport { sent: peers_reached > 0, peers_reached, error: None }
                }
            }
        }
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::VoiceError;
use rekindle_codec::capnp_codec::SignedWire;

pub mod allocation;
pub mod egress;
pub mod link;
mod packet;
pub mod roster;

use link::PeerLink;
use roster::MediaRoster;

pub use packet::{OutboundFrame, VoicePacket};

/// One-byte media tag a voice packet travels under on `app_message`, so
/// ingress can tell it from envelopes and receiver reports (`b'R'`).
pub const VOICE_PACKET_TAG: u8 = crate::media_frame::VOICE_TAG;

/// Voice channel operating mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceMode {
    /// Full mesh: each participant sends to every other.
    /// Suitable for 2-5 participants.
    #[default]
    Mesh,
    /// MCU mode: one peer (host) receives all, mixes, redistributes.
    /// Suitable for 6-15+ participants.
    Mcu {
        /// Pseudonym key (hex) of the mixing host.
        host_pseudonym: String,
    },
}

/// IO port for shipping a finished voice packet to a peer's Veilid
/// private route (architecture §14 `VoiceSessionDeps::send_voice_frame`).
///
/// Implemented in the `src-tauri` voice adapter — the sanctioned layer
/// that owns `veilid-core`. Injected into [`VoiceTransport`] so
/// `rekindle-voice` stays free of `veilid-core` (Invariant 2 — only
/// `rekindle-transport` and `rekindle-protocol` may import it). The
/// adapter imports the route (cached) and calls Veilid `app_message`
/// over a 3-hop Tor-class `SafetySelection::Safe` route (LowLatency
/// stability) — anonymous on every voice frame, never Unsafe.
#[async_trait]
pub trait VoiceFrameSender: Send + Sync {
    /// Ship already-built wire bytes (a signed SFrame packet, `b'V'`-tagged)
    /// to the peer reachable via `route_blob`.
    async fn send_voice_frame(&self, route_blob: &[u8], data: Vec<u8>) -> Result<(), VoiceError>;
}

/// Multi-peer voice transport over the Veilid network.
///
/// Supports both full-mesh (each peer sends to every other) and MCU
/// (all peers send to one host, host mixes and redistributes) modes.
///
/// All network IO is delegated to an injected [`VoiceFrameSender`],
/// which routes voice over a 3-hop Tor-class `SafetySelection::Safe`
/// route (LowLatency stability) — the lowest-latency variant that still
/// hides the sender, accepting a modest latency cost for anonymity.
pub struct VoiceTransport {
    channel_id: String,
    sender: Option<Arc<dyn VoiceFrameSender>>,
    sender_key: Vec<u8>,
    /// Architecture §10.3 + §26 W26 — pseudonym signing key for voice
    /// packet authentication. Derived from the user's identity secret +
    /// community_id (or identity secret alone for 1:1 calls). Cleared on
    /// disconnect so a stale key can't sign packets after channel exit.
    signing_key: Option<ed25519_dalek::SigningKey>,
    /// Connected peers: pseudonym_key (hex) → roster entry.
    peers: HashMap<String, VoicePeer>,
    /// Current operating mode.
    mode: VoiceMode,
    /// Local three-way join handshake progress.
    handshake: JoinHandshake,
    /// Every peer's media link and the allocator over them (plan E4.3.3),
    /// shared with producers that must not wait on this transport's lock.
    media: Arc<MediaRoster>,
    /// Scope the per-peer egress drivers run in; set by `init`.
    scope: Option<Arc<rekindle_lifecycle::SessionScope>>,
}

/// Whether a per-peer send error is a Veilid `NoConnection`.
///
/// `rekindle-voice` cannot see `veilid_core::VeilidAPIError` (Invariant
/// 2), and the frame-sender adapter wraps it as
/// `VoiceError::Transport("app_message: {e}")`, where `NoConnection`'s
/// `Display` is `"No connection: {message}"`. Matching that substring is
/// the only signal available on this side of the boundary. It gates a
/// log warning only — never control flow or the send return contract —
/// so a false negative merely misses a log line, never a dropped frame.
fn is_no_connection(err: &VoiceError) -> bool {
    matches!(err, VoiceError::Transport(msg) if msg.contains("No connection"))
}

/// One connected roster entry. `added_at` powers the presence-reconcile
/// join-grace: a peer added moments ago via gossip must not be expired
/// just because their presence row hasn't propagated yet.
#[derive(Clone)]
pub struct VoicePeer {
    /// When this entry was (first) added to the roster.
    pub added_at: std::time::Instant,
    /// Display name as carried by the join handshake (VoiceJoin /
    /// VoiceJoinAck / roster entry). `None` until any leg supplies it.
    pub display_name: Option<String>,
    /// The peer's media link: the route it advertised (VoiceJoin /
    /// roster / re-resolve), its bandwidth owner and egress driver (plan
    /// E4.3.3).
    pub link: Arc<PeerLink>,
}

/// Local three-way join handshake progress (SimpleX
/// `x.grp.acpt`/`x.grp.mem.con` analog): we announced (leg 1), a
/// member saw us (leg 2 — explicit VoiceJoinAck, or implicit via a
/// roster/mutual VoiceJoin), and we confirmed transport-readiness
/// (leg 3 — VoiceJoinConfirmed sent). A member alone in a channel
/// stays `Announced` until a second participant completes the
/// handshake mutually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JoinHandshake {
    /// VoiceJoin broadcast, no evidence anyone received it.
    #[default]
    Announced,
    /// At least one member acked / showed up — we are seen.
    Seen,
    /// VoiceJoinConfirmed sent — both sides converged.
    Connected,
}

impl VoiceTransport {
    /// Create a new transport for a voice channel.
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            sender: None,
            sender_key: Vec::new(),
            signing_key: None,
            peers: HashMap::new(),
            mode: VoiceMode::default(),
            handshake: JoinHandshake::default(),
            media: Arc::default(),
            scope: None,
        }
    }

    /// Current local join-handshake stage.
    pub fn handshake(&self) -> JoinHandshake {
        self.handshake
    }

    /// Leg-2 evidence arrived (explicit VoiceJoinAck, roster receipt,
    /// or a mutual VoiceJoin while we were still unseen). Returns
    /// `true` only on the Announced→Seen transition so callers emit
    /// the state event exactly once.
    pub fn advance_handshake_seen(&mut self) -> bool {
        if self.handshake == JoinHandshake::Announced {
            self.handshake = JoinHandshake::Seen;
            return true;
        }
        false
    }

    /// Leg 3 — mark the handshake complete. Returns `true` only on
    /// the first call so VoiceJoinConfirmed is sent exactly once.
    pub fn advance_handshake_connected(&mut self) -> bool {
        if self.handshake != JoinHandshake::Connected {
            self.handshake = JoinHandshake::Connected;
            return true;
        }
        false
    }

    /// The key this transport signs packets with (pseudonym for a
    /// channel, identity for a call).
    pub fn sender_key(&self) -> &[u8] {
        &self.sender_key
    }

    /// Initialize the transport with a frame-sender backend, sender
    /// identity and the scope its per-peer egress drivers run in. After
    /// calling `init()`, add peers with `add_peer()`.
    pub fn init(
        &mut self,
        sender: Arc<dyn VoiceFrameSender>,
        sender_key: Vec<u8>,
        scope: Arc<rekindle_lifecycle::SessionScope>,
    ) {
        self.sender = Some(sender);
        self.sender_key = sender_key;
        self.scope = Some(scope);
        let peers: Vec<(String, Arc<PeerLink>)> = self
            .peers
            .iter()
            .map(|(k, p)| (k.clone(), Arc::clone(&p.link)))
            .collect();
        for (key, link) in peers {
            self.spawn_driver(&key, &link);
        }
    }

    /// The session's media roster: route queues, feedback intake and the
    /// allocator.
    #[must_use]
    pub fn media(&self) -> Arc<MediaRoster> {
        Arc::clone(&self.media)
    }

    /// The session's allocator: encoder targets and keyframe requests.
    #[must_use]
    pub fn allocator(&self) -> Arc<allocation::Allocator> {
        Arc::clone(self.media.allocator())
    }

    /// Start `link`'s egress driver, once the transport has a sender and
    /// a scope.
    fn spawn_driver(&self, key: &str, link: &Arc<PeerLink>) {
        let (Some(sender), Some(scope)) = (self.sender.as_ref(), self.scope.as_ref()) else {
            return;
        };
        let task = Arc::clone(link).drive(
            key.to_string(),
            Arc::clone(sender),
            Arc::clone(self.media.allocator()),
            scope.token(),
        );
        scope.spawn_or_drop("voice media egress", task);
    }

    /// Architecture §26 W26 — install the pseudonym signing key the
    /// transport uses to sign every outbound voice packet. Caller is
    /// responsible for re-installing on community switch (the key is
    /// derived per-community).
    pub fn set_signing_key(&mut self, signing_key: ed25519_dalek::SigningKey) {
        // The same key signs our padding (plan E4.3.3).
        self.media.set_padding_key(Some(signing_key.clone()));
        self.signing_key = Some(signing_key);
    }

    /// Bind a 1:1 DM call's single remote peer (wraps `init` +
    /// `add_peer`).
    ///
    /// `peer_key` is the peer's own identity — the same value inbound
    /// packets carry as `sender_key`, so the roster is addressable by
    /// identity in a DM exactly as it is in a community mesh. One
    /// keying scheme, so anything that learns a peer from a packet
    /// (receiver reports, route healing) can reach it in either
    /// topology.
    pub fn connect(
        &mut self,
        sender: Arc<dyn VoiceFrameSender>,
        route_blob: &[u8],
        sender_key: Vec<u8>,
        peer_key: &str,
        scope: Arc<rekindle_lifecycle::SessionScope>,
    ) {
        self.init(sender, sender_key, scope);
        self.add_peer(peer_key, route_blob, None);
        tracing::info!(
            channel = %self.channel_id,
            peer = %peer_key,
            "voice transport connected (1:1)"
        );
    }

    /// Add a peer to the voice mesh. The route blob is imported lazily
    /// by the [`VoiceFrameSender`] on first send.
    /// Add (or route-upsert) a roster entry. Returns `true` only when
    /// the peer is NEW to the roster — callers emit the roster-changed
    /// signaling event on that edge, so repeat VoiceJoin re-announces
    /// (route refresh) don't re-fire membership downstream.
    pub fn add_peer(
        &mut self,
        pseudonym_key: &str,
        route_blob: &[u8],
        display_name: Option<&str>,
    ) -> bool {
        // Re-add keeps the original added_at (route upsert, not a
        // fresh join) so the presence-reconcile grace isn't reset by
        // repeat VoiceJoin announces. A name supplied by any handshake
        // leg upgrades a missing one; `None` never erases a known name.
        if let Some(existing) = self.peers.get_mut(pseudonym_key) {
            existing.link.set_route(route_blob);
            if let Some(name) = display_name {
                existing.display_name = Some(name.to_string());
            }
            tracing::debug!(
                channel = %self.channel_id,
                peer = %pseudonym_key,
                "voice peer route updated"
            );
            return false;
        }
        tracing::info!(
            channel = %self.channel_id,
            peer = %pseudonym_key,
            "added voice peer"
        );
        let link = PeerLink::new(route_blob, self.media.padding_key());
        self.spawn_driver(pseudonym_key, &link);
        self.media.insert(pseudonym_key, Arc::clone(&link));
        self.peers.insert(
            pseudonym_key.to_string(),
            VoicePeer {
                added_at: std::time::Instant::now(),
                display_name: display_name.map(str::to_string),
                link,
            },
        );
        true
    }

    /// Refresh a peer's route blob ONLY if they are already in the
    /// roster. Healing path for the gossip re-resolve: a successful
    /// DHT re-resolve proves the VoiceJoin-era blob is stale, and
    /// frame sends have no re-resolve of their own. Gossip peers who
    /// are not in this channel must never be added to the media plane,
    /// hence no insert.
    pub fn refresh_peer_route(&mut self, pseudonym_key: &str, route_blob: &[u8]) -> bool {
        match self.peers.get_mut(pseudonym_key) {
            Some(existing) => {
                existing.link.set_route(route_blob);
                tracing::info!(
                    channel = %self.channel_id,
                    peer = %pseudonym_key,
                    "refreshed voice peer route from re-resolve"
                );
                true
            }
            None => false,
        }
    }

    /// `(pseudonym_key, seconds since added)` for every roster entry —
    /// the view the presence-derived roster reconcile decides over.
    pub fn peer_views(&self) -> Vec<(String, u64)> {
        self.peers
            .iter()
            .map(|(k, v)| (k.clone(), v.added_at.elapsed().as_secs()))
            .collect()
    }

    /// `(pseudonym_key, route_blob, display_name)` for every roster
    /// entry — the roster-broadcast builder's view.
    pub fn peer_named_entries(&self) -> Vec<(String, Vec<u8>, Option<String>)> {
        self.peers
            .iter()
            .map(|(k, v)| (k.clone(), v.link.route(), v.display_name.clone()))
            .collect()
    }

    /// Remove a peer from the voice mesh. Returns `true` when the
    /// peer was actually present — callers emit the roster-changed
    /// signaling event on that edge.
    pub fn remove_peer(&mut self, pseudonym_key: &str) -> bool {
        let removed = self.peers.remove(pseudonym_key).is_some();
        if removed {
            self.media.remove(pseudonym_key);
            tracing::info!(
                channel = %self.channel_id,
                peer = %pseudonym_key,
                "removed voice peer"
            );
        }
        removed
    }

    /// Set the voice channel operating mode.
    pub fn set_mode(&mut self, mode: VoiceMode) {
        tracing::info!(channel = %self.channel_id, ?mode, "voice mode changed");
        self.mode = mode;
    }

    /// Get the current voice mode.
    pub fn mode(&self) -> &VoiceMode {
        &self.mode
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Returns the pseudonym keys of all connected peers.
    pub fn peer_keys(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    /// `(pseudonym_key, route_blob)` for every connected peer. The route
    /// is the one the peer advertised in its VoiceJoin — authoritative
    /// for voice. Used by MEK rotation and the roster broadcast to
    /// source routes without depending on the gossip presence overlay
    /// (which lags a fresh join).
    pub fn peer_entries(&self) -> Vec<(String, Vec<u8>)> {
        self.peers
            .iter()
            .map(|(k, v)| (k.clone(), v.link.route()))
            .collect()
    }

    /// Queue an encoded audio frame for ALL connected peers (mesh mode),
    /// at the head of each route's pacer.
    ///
    /// # Errors
    /// The frame cannot be signed (no signing key installed).
    pub fn broadcast(&self, frame: &OutboundFrame) -> Result<(), VoiceError> {
        let payload: Arc<[u8]> = Arc::from(self.build_packet_payload(frame)?);
        self.media
            .send_unpaced_to_all(crate::media_frame::VOICE_TAG, &payload, frame.media_bytes);
        Ok(())
    }

    /// Datagrams sent and failed over every route so far.
    #[must_use]
    pub fn send_counts(&self) -> (u64, u64) {
        self.media.send_counts()
    }

    /// Queue an encoded audio frame for one peer (MCU mode).
    ///
    /// # Errors
    /// The peer is not on the roster, or the frame cannot be signed.
    pub fn send_to_peer(
        &self,
        pseudonym_key: &str,
        frame: &OutboundFrame,
    ) -> Result<(), VoiceError> {
        let payload = self.build_packet_payload(frame)?;
        let peer = self
            .peers
            .get(pseudonym_key)
            .ok_or_else(|| VoiceError::Transport(format!("peer not found: {pseudonym_key}")))?;
        peer.link.enqueue_unpaced(
            crate::media_frame::VOICE_TAG,
            Arc::from(payload),
            frame.media_bytes,
        );
        Ok(())
    }

    /// Ship already-built wire bytes to one peer's cached route.
    ///
    /// Unlike [`Self::send_to_peer`] this neither builds nor signs a
    /// packet — the caller owns the payload. Used for receiver reports,
    /// which are signed by the *receiving* side under their own domain
    /// tag and so cannot go through the packet builder. Reuses the
    /// cached peer route, so a report costs no DHT lookup and travels
    /// the same media route as the audio it describes.
    pub async fn send_bytes_to_peer(
        &self,
        pseudonym_key: &str,
        data: Vec<u8>,
    ) -> Result<(), VoiceError> {
        let peer = self
            .peers
            .get(pseudonym_key)
            .ok_or_else(|| VoiceError::Transport(format!("peer not found: {pseudonym_key}")))?;
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| VoiceError::Transport("transport not initialized".into()))?;
        sender.send_voice_frame(&peer.link.route(), data).await
    }

    /// Queue a frame for the whole roster.
    ///
    /// This is the send loop's normal path in both topologies: a DM
    /// roster holds one peer, a mesh roster holds all of them. Delivery
    /// happens in each peer's egress driver, so one dead route never holds
    /// up the others.
    ///
    /// # Errors
    /// No peers, or the frame cannot be signed.
    pub fn send(&self, frame: &OutboundFrame) -> Result<(), VoiceError> {
        if self.peers.is_empty() {
            return Err(VoiceError::NotConnected);
        }
        self.broadcast(frame)
    }

    /// Disconnect from the voice channel — removes all peers.
    pub fn disconnect(&mut self) {
        for (key, _) in self.peers.drain() {
            self.media.remove(&key);
        }
        self.sender = None;
        self.sender_key.clear();
        self.signing_key = None;
        self.media.set_padding_key(None);
        self.mode = VoiceMode::default();
        self.handshake = JoinHandshake::default();
        tracing::info!(channel = %self.channel_id, "voice transport disconnected");
    }

    /// Decode an incoming voice packet (bytes after the `b'V'` tag) and
    /// verify its signature against `sender_key`. Decryption happens in
    /// the receiving loop, which knows the session's keys.
    pub fn receive(data: &[u8]) -> Result<VoicePacket, VoiceError> {
        let packet = VoicePacket::decode(data).map_err(|e| VoiceError::Transport(e.to_string()))?;
        packet
            .verify()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        Ok(packet)
    }

    /// Whether the transport has any connected peers.
    pub fn is_connected(&self) -> bool {
        !self.peers.is_empty()
    }

    /// The channel ID this transport is for.
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// Sign `frame` as a packet from this transport's sender. The per-route
    /// framing (tag and `transport_seq`) is added at egress.
    fn build_packet_payload(&self, frame: &OutboundFrame) -> Result<Vec<u8>, VoiceError> {
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or_else(|| VoiceError::Transport("voice signing key not installed".into()))?;
        let mut packet = VoicePacket {
            sender_key: self.sender_key.clone(),
            sequence: frame.sequence,
            timestamp: frame.timestamp,
            sframe: frame.sframe.clone(),
            sig: Vec::new(),
        };
        packet.sign(signing_key);
        Ok(packet.encode())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

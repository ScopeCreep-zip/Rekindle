//! The session's media roster: every peer's [`PeerLink`] and the
//! [`Allocator`] over them (plan E4.3.3).
//!
//! Producers reach the routes through this without the transport's async
//! lock, which the voice send loop holds per frame. A video frame has to
//! land on every route queue in the order it was encoded, so its path is
//! synchronous end to end; going through the async lock would mean one
//! spawned task per frame, and those can run out of order.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;

use super::allocation::{Allocator, RouteAllocation};
use super::egress::VideoFrame;
use super::link::PeerLink;
use crate::arrivals::ArrivalLedger;
use crate::media_frame;
use crate::media_quality::MediaQuality;

/// The key padding datagrams are signed with: the transport's signing
/// key, installed after the roster may already hold peers.
pub type PaddingKey = Arc<RwLock<Option<ed25519_dalek::SigningKey>>>;

#[derive(Default)]
pub struct MediaRoster {
    links: RwLock<HashMap<String, Arc<PeerLink>>>,
    allocator: Arc<Allocator>,
    /// What arrived from each peer, for the feedback we send it.
    arrivals: Arc<ArrivalLedger>,
    /// What the call looks and sounds like here (plan E4.3 Q0).
    quality: Arc<MediaQuality>,
    padding_key: PaddingKey,
}

impl MediaRoster {
    /// Arrivals from each peer, recorded once a datagram proved fresh.
    #[must_use]
    pub fn arrivals(&self) -> &Arc<ArrivalLedger> {
        &self.arrivals
    }

    /// Render, post-FEC loss and lip-sync measurement (plan E4.3 Q0).
    #[must_use]
    pub fn quality(&self) -> &Arc<MediaQuality> {
        &self.quality
    }

    pub(super) fn padding_key(&self) -> PaddingKey {
        Arc::clone(&self.padding_key)
    }

    pub(super) fn set_padding_key(&self, key: Option<ed25519_dalek::SigningKey>) {
        *self.padding_key.write() = key;
    }

    pub(super) fn insert(&self, peer: &str, link: Arc<PeerLink>) {
        self.links.write().insert(peer.to_string(), link);
    }

    /// Take `peer` off the roster: its driver stops and its share of the
    /// allocation is forgotten.
    pub(super) fn remove(&self, peer: &str) {
        if let Some(link) = self.links.write().remove(peer) {
            link.stop();
        }
        self.allocator.forget(peer);
    }

    /// `peer`'s link, if on the roster.
    #[must_use]
    pub fn link(&self, peer: &str) -> Option<Arc<PeerLink>> {
        self.links.read().get(peer).cloned()
    }

    /// The session's allocator: encoder targets and keyframe requests.
    #[must_use]
    pub fn allocator(&self) -> &Arc<Allocator> {
        &self.allocator
    }

    /// Queue an unpaced datagram (`tag`) for every peer.
    pub fn send_unpaced_to_all(&self, tag: u8, payload: &Arc<[u8]>, media_bytes: usize) {
        for link in self.links.read().values() {
            link.enqueue_unpaced(tag, Arc::clone(payload), media_bytes);
        }
    }

    /// Queue a signed video-plane control envelope (keyframe request,
    /// topology change) for every peer, unpaced: control rides ahead of
    /// media, as RTCP does in libwebrtc.
    pub fn send_control_envelope(&self, signed_envelope: &[u8]) {
        let payload: Arc<[u8]> = Arc::from(signed_envelope);
        self.send_unpaced_to_all(media_frame::ENVELOPE_TAG, &payload, 0);
    }

    /// Queue a video frame's signed fragments on every peer's paced queue.
    /// Returns how many routes took it (a paused route refuses it).
    pub fn send_video_frame(&self, frame: &VideoFrame) -> usize {
        self.links
            .read()
            .values()
            .filter(|link| link.enqueue_video(frame))
            .count()
    }

    /// `peer`'s feedback about the media we send it: the route's estimate,
    /// split between audio and video, feeds the encoder targets. Feedback
    /// from a peer not on the roster is dropped (r6 R-BW6).
    pub fn on_transport_feedback(
        &self,
        peer: &str,
        feedback: &TransportFeedback,
    ) -> Option<RouteAllocation> {
        let link = self.link(peer)?;
        let split = link.on_feedback(feedback);
        self.allocator.note(peer, split);
        Some(split)
    }

    /// Datagrams sent and failed over every route so far.
    #[must_use]
    pub fn send_counts(&self) -> (u64, u64) {
        self.links.read().values().fold((0, 0), |(s, f), link| {
            let (ls, lf) = link.send_counts();
            (s + ls, f + lf)
        })
    }
}

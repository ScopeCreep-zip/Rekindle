//! Transport feedback from a peer about the media we send it (plans
//! E4.3.2, E4.3.3): handed to that peer's route controller, whose new
//! estimate the allocator divides between audio and video.

use std::sync::Arc;
use std::time::Instant;

use rekindle_codec::capnp_codec::transport_feedback::TransportFeedback;

use crate::state::AppState;

/// A verified report: accepted only from a peer on our voice roster,
/// about its own reception (r6 R-BW6).
pub(crate) fn on_transport_feedback(
    state: &Arc<AppState>,
    feedback: &TransportFeedback,
    arrived: Instant,
) {
    let Some(media) = ({
        let engine = state.voice_engine.lock();
        engine.as_ref().map(|h| Arc::clone(&h.media))
    }) else {
        return;
    };
    let reporter = hex::encode(&feedback.reporter_key);
    let Some(split) = media.on_transport_feedback(&reporter, feedback) else {
        tracing::debug!(peer = %reporter,
            "transport feedback from a peer not on our roster — dropped");
        return;
    };
    // A roster peer reporting on our media is alive even when silent.
    crate::state_helpers::note_media_live(state, &reporter);
    tracing::debug!(
        peer = %reporter,
        reported = feedback.arrivals.len(),
        estimate_kbps = split.estimate_bps / 1_000,
        audio_bps = split.audio_bps,
        video_bps = split.video_bps,
        feedback_age_ms = arrived.elapsed().as_millis(),
        "transport feedback"
    );
}

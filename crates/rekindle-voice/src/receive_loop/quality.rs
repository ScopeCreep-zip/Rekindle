//! The 5-second window cadence: MEK-drop accounting and the quality /
//! receiver-report pass.
//!
//! Split from the loop body because it is the only part that runs on a
//! cadence rather than per packet, and because it is where the loop
//! talks *back* — the RFC 3550 receiver reports that let each sender
//! see what its stream looks like from here.
//!
//! ## On the report bandwidth
//!
//! RFC 3550 §6.2 caps control traffic at 5 % of session bandwidth and
//! scales its interval with participant count to hold that. We use a
//! fixed 5 s instead, and it fits: one ~150-byte report per peer per
//! 5 s is 30 B/s against a 32 kbps (4 kB/s) per-peer audio stream —
//! about 0.75 %. The N² shape is the mesh's, not the reports': the
//! audio itself already goes to every peer, and above four
//! participants the topology switches to an SFU. So the interval does
//! not need to scale, and shortening it would be the change that
//! needed justifying, not lengthening it.

use std::time::{Duration, Instant};

use super::VoiceReceiveLoop;
use crate::receiver_report::VoiceReceiverReport;
use crate::session_deps::VoiceSessionEvent;

impl VoiceReceiveLoop {
    /// Count an undecryptable packet and (debounced, 10s) fire the
    /// RequestMEK cascade. Drops are surfaced in ReceiveStats — a
    /// security-relevant drop must never be silent.
    pub(super) fn note_mek_drop(
        &mut self,
        community_id: &str,
        channel_id: &str,
        reason: &'static str,
        needed_generation: u64,
    ) {
        self.mek_drops += 1;
        self.deps.record_packet_drop();
        let due = self
            .last_mek_request
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(10));
        if due {
            tracing::info!(community = %community_id, channel = %channel_id, reason,
                needed_generation, "requesting channel MEK refresh");
            self.deps
                .request_mek_refresh(community_id, channel_id, needed_generation);
            self.last_mek_request = Some(Instant::now());
        }
    }

    pub(super) fn log_quality_if_due(&mut self) {
        if self.last_quality_check.elapsed() < Duration::from_secs(5) {
            return;
        }
        // Phase 5 — surface receive-side jitter drops (overflow trims +
        // late arrivals) so Linux dropouts are attributable from the UI
        // instead of trace-level logs.
        let now_local_ms = self.local_ms();
        let reporter_key = self.our_key_bytes.clone();
        let signing_key = self.report_signing_key.clone();

        let (mut overflow, mut late) = (0u64, 0u64);
        let mut reports: Vec<(String, Vec<u8>)> = Vec::new();
        for (peer_key, participant) in &mut self.participants {
            let (o, l) = participant.jitter_buffer.take_drops();
            // Late drops in this window mean the adaptive target was too
            // low — grow it; a clean window advances toward the shrink
            // gate. (Per-push EWMA handles fast jitter; this is the
            // slow safety net + controlled shrink.)
            participant.jitter_buffer.note_window_health(l);
            overflow += o;
            late += l;

            // RFC 3550 receiver report — the return path voice never
            // had. Without it a sender sees only its own successful
            // `send()` calls and cannot tell a clean link from one
            // dropping a fifth of its packets.
            if let Some(sk) = signing_key.as_ref() {
                let wire = VoiceReceiverReport::build_signed(
                    sk,
                    reporter_key.clone(),
                    &participant.echo,
                    participant.jitter_buffer.reception_metrics(),
                    participant.jitter_buffer.target_delay_ms(),
                    now_local_ms,
                )
                .and_then(|r| r.to_wire().ok());
                if let Some(wire) = wire {
                    reports.push((hex::encode(peer_key), wire));
                }
            }
        }
        // Sent outside the loop: `send_receiver_report` is the adapter's
        // I/O port, and the participant map is mutably borrowed above.
        //
        // Logged at `info` because "no reports arrived" is ambiguous
        // between the two ends — this line is how you tell a receiver
        // that never sent from a sender that never received, which
        // otherwise needs both machines' logs side by side to diagnose.
        if !reports.is_empty() {
            tracing::info!(
                count = reports.len(),
                participants = self.participants.len(),
                "voice link: dispatching receiver reports"
            );
        } else if !self.participants.is_empty() && signing_key.is_none() {
            tracing::warn!(
                participants = self.participants.len(),
                "voice link: no signing identity — sending no receiver reports, so our peers \
                 cannot measure the stream we send them"
            );
        }
        for (peer_hex, wire) in reports {
            self.deps.send_receiver_report(&peer_hex, wire);
        }
        if overflow > 0 || late > 0 {
            tracing::warn!(
                rx_overflow_drops = overflow,
                rx_late_drops = late,
                "voice receive-side drops in the last 5s"
            );
        }
        let mek_drops = std::mem::take(&mut self.mek_drops);
        if mek_drops > 0 {
            tracing::warn!(
                rx_mek_drops = mek_drops,
                "voice packets dropped for MEK reasons in the last 5s"
            );
        }
        self.deps.emit_voice_event(VoiceSessionEvent::ReceiveStats {
            rx_overflow_drops: overflow,
            rx_late_drops: late,
            rx_mek_drops: mek_drops,
        });
        tracing::debug!(
            participants = self.participants.len(),
            self.packets_received,
            "voice receive loop stats"
        );
        self.packets_received = 0;
        self.last_quality_check = Instant::now();
    }
}

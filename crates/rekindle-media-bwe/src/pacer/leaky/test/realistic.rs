//! The realistic pacer test (str0m `src/pacer/leaky.rs`, `mod test`).

use super::*;
use crate::util::{f64_to_u64, u64_as_usize, usize_as_i64};

/// `v as f32` (str0m): padding requests are a few kB, well inside `u16`, where
/// the conversion is exact.
fn usize_as_f32(v: usize) -> f32 {
    f32::from(u16::try_from(v).expect("padding request fits u16"))
}

/// `v as usize` (str0m): `f32` widens to `f64` exactly, then truncates.
fn f32_as_usize(v: f32) -> usize {
    u64_as_usize(f64_to_u64(f64::from(v)))
}

#[test]
fn test_realistic() {
    let config = RealisticTestConfig {
        padding_rate: Bitrate::kbps(2500),
        max_overshoot_factor: 0.05,
        spike_probability: 3,
        ..Default::default()
    };
    let (media_rate, padding_rate, total_rate) = run_realistic_test(config);
    let expected_padding = config.padding_rate - config.media_rate;
    // Expect result to be within 2 standard deviations.
    let upper_bound =
        config.media_rate + expected_padding * f64::from(1.0 + config.max_overshoot_factor * 2.0);
    let lower_bound =
        config.media_rate + expected_padding * f64::from(1.0 - config.max_overshoot_factor * 2.0);

    assert!(
        total_rate >= lower_bound && total_rate <= upper_bound,
        "Expected reuslting total rate to be within expected bounds. \
        total_rate={total_rate}, media_rate={media_rate}, padding_rate={padding_rate}, \
        config={config:?}, lower_bound={lower_bound}, upper_bound={upper_bound}"
    );
}

#[derive(Debug, Clone, Copy)]
struct RealisticTestConfig {
    media_rate: Bitrate,
    padding_rate: Bitrate,
    duration: Duration,
    // Spike probability as a percentage
    spike_probability: u8,
    max_overshoot_factor: f32,
    frame_pacing: Duration,
}

impl Default for RealisticTestConfig {
    fn default() -> Self {
        RealisticTestConfig {
            media_rate: Bitrate::kbps(250),
            padding_rate: Bitrate::kbps(800),
            duration: Duration::from_secs(10),
            spike_probability: 0,
            max_overshoot_factor: 0.25,
            frame_pacing: Duration::from_millis(33), // ~30 FPS
        }
    }
}

/// Run a realistic test of the pacer with simulated media.
///
/// Returns the media rate, padding, rate, and total rate achieved by the test.
fn run_realistic_test(config: RealisticTestConfig) -> (Bitrate, Bitrate, Bitrate) {
    let RealisticTestConfig {
        media_rate,
        padding_rate,
        duration,
        spike_probability,
        max_overshoot_factor,
        frame_pacing,
    } = config;

    let base = Instant::now();
    let mut queue = Queue::default();
    let mut pacer = LeakyBucketPacer::new(media_rate);
    pacer.set_pacing_rate(padding_rate);
    pacer.set_padding_rate(padding_rate);

    let mut last_media_at = base
        .checked_sub(frame_pacing)
        .and_then(|t| t.checked_sub(Duration::from_millis(1)))
        .unwrap();
    let mut media_sent = DataSize::ZERO;
    let mut padding_sent = DataSize::ZERO;
    let mut elapsed = Duration::ZERO;

    let generate_padding = |queue: &mut Queue, now: Instant, request: PaddingRequest| {
        let rand: f32 = fastrand::f32();
        let overshoot_factor: f32 = rand * max_overshoot_factor;
        let final_size =
            f32_as_usize(usize_as_f32(request.padding) * (1.0 + overshoot_factor).round());
        queue.generate_padding(final_size, now);
    };

    loop {
        if elapsed > duration {
            break;
        }

        let timeout = {
            if let Some((queue_id, _cluster_id)) = pacer.poll_queue() {
                let packet = queue
                    .next_packet()
                    .unwrap_or_else(|| panic!("Should have a packet for {queue_id:?}"));
                queue.register_send(queue_id, base + elapsed);
                queue.update_average_queue_time(base + elapsed);
                pacer.register_send(
                    base + elapsed,
                    DataSize::bytes(usize_as_i64(packet.payload_len)),
                    queue_id,
                );
                if packet.kind == PacketKind::Padding {
                    padding_sent += packet.payload_len.into();
                } else {
                    media_sent += packet.payload_len.into();
                }
                continue;
            }

            pacer.poll_timeout()
        };

        let sleep_until_poll = timeout
            .0
            .map_or(Duration::ZERO, |t| t.duration_since(base + elapsed));

        let sleep_until_media =
            frame_pacing.saturating_sub((base + elapsed).duration_since(last_media_at));

        if sleep_until_poll < sleep_until_media {
            elapsed += sleep_until_poll;

            queue.update_average_queue_time(base + elapsed);
            if let Some(padding_request) =
                pacer.handle_timeout(base + elapsed, queue.queue_state(base + elapsed))
            {
                generate_padding(&mut queue, base + elapsed, padding_request);
            }
            continue;
        }
        elapsed += sleep_until_media;

        let large_overshoot = (fastrand::u8(..) % 100) >= (100 - spike_probability);
        let mut to_add = if large_overshoot {
            (media_rate * 2.5) * frame_pacing
        } else {
            media_rate * frame_pacing
        };

        while to_add > DataSize::ZERO {
            let packet_size = to_add.min(DataSize::bytes(1100));
            let (header, size, kind) =
                make_packet(0, packet_size.as_bytes_usize(), PacketKind::Video);
            queue.enqueue_packet(QueuedPacket {
                queued_at: base + elapsed,
                header,
                payload_len: size,
                kind,
            });
            to_add -= packet_size;
        }
        last_media_at = base + elapsed;
    }

    let observed_media_rate = media_sent / duration;
    let observed_padding_rate = padding_sent / duration;
    let total_rate = (media_sent + padding_sent) / duration;

    (observed_media_rate, observed_padding_rate, total_rate)
}

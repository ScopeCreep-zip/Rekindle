use super::*;
use crate::reassembly_state::VideoReassemblyState;
use crate::test_mock::MockDeps;
use rekindle_codec::community::envelope::ControlPayload;

fn small_request(keyframe: bool) -> VideoFrameSend {
    VideoFrameSend {
        stream_id: [9u8; 16],
        frame_seq: 1,
        keyframe,
        codec: Codec::Vp9,
        timestamp: 100,
        encoded_payload: vec![0xAB; 256],
    }
}

#[test]
fn empty_payload_rejected() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let mut req = small_request(false);
    req.encoded_payload.clear();
    let err = build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &req,
        0,
    )
    .unwrap_err();
    assert!(matches!(err, VideoError::InvalidInput(_)));
}

#[test]
fn missing_mek_rejected() {
    let deps = MockDeps::without_mek();
    let reassembly = VideoReassemblyState::new();
    let err = build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &small_request(false),
        0,
    )
    .unwrap_err();
    assert!(matches!(err, VideoError::MekUnavailable { .. }));
}

#[test]
fn missing_identity_rejected() {
    let deps = MockDeps::without_signing_key();
    let reassembly = VideoReassemblyState::new();
    let err = build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &small_request(false),
        0,
    )
    .unwrap_err();
    assert!(matches!(err, VideoError::IdentityNotLoaded));
}

/// Phase F smoke test — install a process-wide `tracing` Layer that
/// records every event's target + field set, then exercise
/// `send_video_frame`. Confirms the `rekindle_video::send` target
/// with structured fields actually fires (which is what
/// `RUST_LOG=rekindle_video=debug` operators see at runtime).
///
/// **Why global, not per-thread.** `tracing` caches per-callsite
/// `Interest` decisions on first registration. If parallel sibling
/// tests reach the same callsite first with no subscriber, the
/// macro is cached as "never enabled" for the entire process and
/// later per-thread `with_default` subscribers can't observe the
/// event. Setting one global Layer up-front via `Once` makes the
/// callsites permanently enabled for the test binary.
#[test]
fn structured_trace_emits_on_send() {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::Registry;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    #[derive(Debug, Default)]
    struct Captured {
        target: String,
        message: String,
        fields: Vec<String>,
    }

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<Captured>>>);

    impl<S: tracing::Subscriber> Layer<S> for Sink {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            let mut rec = Captured {
                target: event.metadata().target().to_string(),
                ..Captured::default()
            };
            struct V<'a>(&'a mut Captured);
            impl Visit for V<'_> {
                fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                    let formatted = format!("{value:?}");
                    if field.name() == "message" {
                        self.0.message = formatted;
                    } else {
                        self.0.fields.push(field.name().to_string());
                    }
                }
            }
            event.record(&mut V(&mut rec));
            self.0.lock().push(rec);
        }
    }

    // One global sink shared by every test; the smoke test reads
    // its own slice by filtering on the unique community_id we use
    // for `send_video_frame` (sentinel: `"c_smoke_phase_f"`).
    //
    // We also chain an stderr-fmt layer so the Phase F smoke
    // verification shell command
    //   RUST_LOG=… cargo test -p rekindle-video --lib send 2>&1 \
    //     | grep "rekindle_video::send"
    // observes the same structured output the assertions below
    // verify. The fmt layer respects `RUST_LOG`; the sink layer
    // does not — assertions stay deterministic regardless of env.
    static SINK: std::sync::OnceLock<Sink> = std::sync::OnceLock::new();
    let sink = SINK
        .get_or_init(|| {
            use tracing_subscriber::EnvFilter;
            let s = Sink::default();
            let fmt_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(
                    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("off")),
                );
            let _ = Registry::default()
                .with(s.clone())
                .with(fmt_layer)
                .try_init();
            s
        })
        .clone();

    let snapshot_before = sink.0.lock().len();
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let mut req = small_request(false);
    // Distinct stream_id so our IPC-entry trace is identifiable
    // even if a parallel test fires the same callsite.
    req.stream_id = [0xF0; 16];
    build_video_frame(
        &deps,
        &reassembly,
        "c_smoke_phase_f",
        "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e",
        &req,
        0,
    )
    .expect("build happy path");

    let captured = sink.0.lock();
    let new_events: Vec<&Captured> = captured
        .iter()
        .skip(snapshot_before)
        .filter(|c| c.target == "rekindle_video::send")
        .collect();
    assert!(
        new_events.len() >= 2,
        "expected at least two `rekindle_video::send` events (IPC entry + post-send), got {}: {:?}",
        new_events.len(),
        new_events
            .iter()
            .map(|c| (&c.message, &c.fields))
            .collect::<Vec<_>>()
    );
    let has_frame_seq = new_events
        .iter()
        .any(|c| c.fields.iter().any(|f| f == "frame_seq"));
    let has_stream_id = new_events
        .iter()
        .any(|c| c.fields.iter().any(|f| f == "stream_id"));
    let has_fragment_count = new_events
        .iter()
        .any(|c| c.fields.iter().any(|f| f == "fragment_count"));
    let has_encoded_bytes = new_events
        .iter()
        .any(|c| c.fields.iter().any(|f| f == "encoded_bytes"));
    assert!(
        has_frame_seq && has_stream_id,
        "send events must carry frame_seq and stream_id; got: {new_events:?}"
    );
    assert!(
        has_fragment_count,
        "at least one send event must carry fragment_count; got: {new_events:?}"
    );
    assert!(
        has_encoded_bytes,
        "IPC-entry event must carry encoded_bytes; got: {new_events:?}"
    );
}

#[test]
fn first_frame_emits_initial_topology_change() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    let frame = build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &small_request(false),
        7,
    )
    .expect("build happy path");
    assert!(!frame.envelopes.is_empty(), "at least one fragment");
    assert_eq!(frame.enqueued_ms, 7);
    // The pacer-bypassing control envelope: exactly the initial
    // TopologyChange went straight to deps; the fragments did NOT.
    let calls = deps.calls.lock();
    assert_eq!(
        calls.sent.len(),
        1,
        "only the topology envelope dispatches here"
    );
    assert!(matches!(
        calls.sent.first().expect("topology sent"),
        CommunityEnvelope::Control(ControlPayload::TopologyChange { reason, .. }) if reason == "initial"
    ));
    // The returned frame carries the fragments for the pacer.
    for env in &frame.envelopes {
        assert!(matches!(
            env,
            CommunityEnvelope::Control(ControlPayload::VideoFragment(_))
        ));
    }
}

#[test]
fn second_frame_same_stream_skips_initial_topology() {
    let deps = MockDeps::new();
    let reassembly = VideoReassemblyState::new();
    build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &small_request(false),
        0,
    )
    .unwrap();
    let mut req2 = small_request(false);
    req2.frame_seq = 2;
    let frame2 = build_video_frame(
        &deps,
        &reassembly,
        "c1",
        "11111111111111111111111111111111",
        &req2,
        0,
    )
    .unwrap();
    // The second build must NOT dispatch another TopologyChange.
    let calls = deps.calls.lock();
    let topology_count = calls
        .sent
        .iter()
        .filter(|e| {
            matches!(
                e,
                CommunityEnvelope::Control(ControlPayload::TopologyChange { .. })
            )
        })
        .count();
    assert_eq!(
        topology_count, 1,
        "TopologyChange fires exactly once per stream"
    );
    assert!(
        !frame2.envelopes.is_empty(),
        "second build produces at least 1 fragment"
    );
}

#[test]
fn parity_count_for_multi_shard_inter_frame_is_positive() {
    // Deltas get parity too now — a lost delta fragment costs a
    // keyframe request, which is far pricier than 25% parity.
    let ct = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * 2 + 100];
    assert!(parity_count_for(&ct) >= 1);
}

#[test]
fn parity_count_for_single_shard_frame_is_zero() {
    let ct = vec![0u8; 100]; // < FRAGMENT_PAYLOAD_LIMIT
    assert_eq!(parity_count_for(&ct), 0);
}

#[test]
fn parity_count_for_multi_shard_keyframe_is_positive() {
    let ct = vec![0u8; FRAGMENT_PAYLOAD_LIMIT * 4 + 100];
    let p = parity_count_for(&ct);
    assert!(
        p >= 1,
        "expected at least 1 parity shard for 5-shard keyframe"
    );
}

#[test]
fn canonical_keyframe_gets_quarter_parity() {
    // 24 KiB keyframe at the 4 KiB budget: 6 data shards → 2 parity
    // (div_ceil(6/4)) — the R4 budget-model mix in the plan.
    let ct = vec![0u8; 24 * 1024];
    assert_eq!(parity_count_for(&ct), 2);
}

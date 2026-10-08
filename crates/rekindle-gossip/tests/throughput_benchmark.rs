//! Architecture §32 Phase 7 Week 26 — gossip throughput perf target:
//! "100 messages/second sustained" (line 4144).
//!
//! We measure the local sign-and-dedup pipeline: build N signed
//! envelopes, push each through `DedupCache::check_and_insert`, and
//! report the elapsed wall time. The actual network fan-out is a
//! transport-layer property not testable in unit tests.
//!
//! Target: ≥100 envelopes/second sustained. We assert at 100 since
//! anything below that violates the spec target. CI runs in debug
//! mode where signing is ~3-5× slower than release, so we run a
//! smaller batch (300 envelopes) and amortise.

use rekindle_codec::dedup::extract_dedup_key;
use rekindle_codec::envelope::build_signed_envelope;
use rekindle_gossip::dedup::DedupCache;
use rekindle_types::governance::GovernanceEntry;

const ENVELOPE_COUNT: usize = 300;
const TARGET_PER_SEC: f64 = 100.0;

#[test]
fn gossip_pipeline_meets_100_per_second_target() {
    let community_id = "perf_community";
    let secret = [0x42_u8; 32];
    let payload = GovernanceEntry::CommunityMeta {
        name: Some("perf".into()),
        description: None,
        icon_hash: None,
        banner_hash: None,
        lamport: 1,
    };

    // Pre-build the dedup cache with realistic capacity so insertion
    // exercises the eviction path on long runs.
    let mut cache = DedupCache::new(1024);

    let started = std::time::Instant::now();
    for i in 0..ENVELOPE_COUNT {
        let bumped = GovernanceEntry::CommunityMeta {
            name: Some(format!("perf-{i}")),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: u64::try_from(i).unwrap() + 1,
        };
        let signed = build_signed_envelope(&secret, community_id, &bumped).unwrap();
        let dedup_key = extract_dedup_key(&signed);
        cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key);
        // Suppress the unused payload reference; it's fine that we
        // build a bumped variant per-iteration.
        let _ = &payload;
    }
    let elapsed = started.elapsed();
    // u32 -> f64 round-trip is exact; ENVELOPE_COUNT is a small const
    // so the conversion never loses precision.
    let count_f64 = f64::from(u32::try_from(ENVELOPE_COUNT).unwrap());
    let envelopes_per_second = count_f64 / elapsed.as_secs_f64();

    assert!(
        envelopes_per_second >= TARGET_PER_SEC,
        "gossip pipeline produced {envelopes_per_second:.1} envelopes/sec — \
         below spec target of {TARGET_PER_SEC}/sec (architecture §32 line 4144). \
         Total: {ENVELOPE_COUNT} envelopes in {elapsed:?}.",
    );
    tracing::info!(
        rate = envelopes_per_second,
        ?elapsed,
        envelopes = ENVELOPE_COUNT,
        "[perf] gossip pipeline throughput",
    );
}

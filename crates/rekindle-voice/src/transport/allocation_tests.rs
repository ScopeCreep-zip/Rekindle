use super::*;

#[test]
fn measured_costs_convert_to_encoder_rates() {
    let share = MediaShare {
        audio_overhead_bytes: Some(1_800.0),
        video_share: Some(0.5),
    };
    // Audio at 64 kbps costs 64k + 50 × 8 × 1800 = 784 kbps of wire.
    let a = allocate(Bitrate::bps(784_000 + 400_000), share, true);
    assert_eq!((a.audio_bps, a.video_bps), (64_000, 200_000));
    assert_eq!(a.estimate_bps, 1_184_000);
    let starved = allocate(Bitrate::kbps(300), share, true);
    assert_eq!((starved.audio_bps, starved.video_bps), (24_000, 100_000));
}

#[test]
fn the_session_encodes_at_the_lowest_route() {
    let alloc = Allocator::new();
    let rx = alloc.subscribe();
    alloc.note(
        "a",
        RouteAllocation {
            estimate_bps: 0,
            audio_bps: 64_000,
            video_bps: 500_000,
            desired_on_wire: 0,
        },
    );
    alloc.note(
        "b",
        RouteAllocation {
            estimate_bps: 0,
            audio_bps: 40_000,
            video_bps: 200_000,
            desired_on_wire: 0,
        },
    );
    assert_eq!(
        *rx.borrow(),
        Allocation {
            audio_bps: 40_000,
            video_bps: 200_000
        },
        "one encoder per kind: the lowest route sets both"
    );
    alloc.forget("b");
    assert_eq!(rx.borrow().audio_bps, 64_000);
}

use super::*;

/// 50 packets a second at 1800 B of overhead each.
const OVERHEAD: Bitrate = Bitrate::bps(720_000);

fn audio() -> AudioCost {
    AudioCost {
        range: Range {
            min: Bitrate::kbps(24),
            max: Bitrate::kbps(64),
        },
        overhead: OVERHEAD,
    }
}

fn video() -> VideoCost {
    VideoCost {
        range: Range {
            min: Bitrate::kbps(100),
            max: Bitrate::kbps(600),
        },
        share: 0.5,
    }
}

fn at(bps: u64) -> Split {
    split(Bitrate::bps(bps), audio(), video(), true)
}

#[test]
fn audio_keeps_its_minimum_below_any_estimate() {
    let s = at(100_000);
    assert_eq!(s.audio, Bitrate::kbps(24));
    assert_eq!(s.video, Bitrate::kbps(100), "video keeps its minimum too");
}

#[test]
fn between_audio_min_and_max_audio_takes_it_all() {
    let s = at(720_000 + 44_000);
    assert_eq!(s.audio, Bitrate::kbps(44));
    assert_eq!(s.video, Bitrate::kbps(100));
}

#[test]
fn audio_fills_first_then_video_takes_the_rest() {
    let s = at(784_000 + 400_000);
    assert_eq!(s.audio, Bitrate::kbps(64));
    assert_eq!(
        s.video,
        Bitrate::kbps(200),
        "400 kbps of wire at a 0.5 share"
    );
    assert_eq!(at(10_000_000).video, Bitrate::kbps(600));
}

#[test]
fn desired_includes_video_only_when_offered() {
    let without = split(Bitrate::kbps(300), audio(), video(), false);
    let with = split(Bitrate::kbps(300), audio(), video(), true);
    assert_eq!(without.desired, Bitrate::bps(784_000));
    assert_eq!(with.desired, Bitrate::bps(784_000 + 1_200_000));
}

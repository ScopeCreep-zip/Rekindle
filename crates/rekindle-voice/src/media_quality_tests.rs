use super::*;

#[test]
fn unwrap_recovers_the_wall_clock() {
    let now = 1_791_442_864_000_u64;
    let sent = now - 250;
    let wire = u32::try_from(sent & u64::from(u32::MAX)).unwrap();
    assert_eq!(unwrap_wall_ms(wire, now), sent);
}

#[test]
fn unwrap_crosses_a_wrap() {
    let now = (5_u64 << 32) + 100;
    let sent = now - 300;
    let wire = u32::try_from(sent & u64::from(u32::MAX)).unwrap();
    assert_eq!(unwrap_wall_ms(wire, now), sent);
}

#[test]
fn audio_and_video_from_one_sender_give_its_skew() {
    let q = MediaQuality::default();
    for i in 0..10 {
        q.note_audio_playout("s", 10_000 + i * 20, 9_880 + i * 20);
        q.note_video_rendered("v", "s", 10_000 + i * 33, 9_840 + i * 33);
    }
    // Audio plays 120 ms after capture, video renders 160 ms after.
    let mut ledger = q.ledger.lock();
    assert_eq!(ledger.sync.get_mut("s").unwrap().sample(), Some(40));
}

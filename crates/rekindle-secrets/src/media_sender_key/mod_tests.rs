use super::*;

#[test]
fn video_frame_keys_differ_per_sender_and_from_the_secret() {
    let secret = fresh();
    let a = video_frame_key(&secret, b"alice");
    let b = video_frame_key(&secret, b"bob");
    assert_ne!(*a, *b);
    assert_ne!(*a, *secret);
}

use super::*;

const CAMERA: &str = "camera";
const SCREEN: &str = "screen";

#[test]
fn camera_and_screen_share_get_distinct_stream_ids() {
    let camera = derive_stream_id("11111111111111111111111111111111", "alice", CAMERA);
    let screen = derive_stream_id("11111111111111111111111111111111", "alice", SCREEN);
    assert_ne!(camera, screen);
}

#[test]
fn same_label_is_deterministic_across_calls() {
    let a = derive_stream_id("11111111111111111111111111111111", "alice", CAMERA);
    let b = derive_stream_id("11111111111111111111111111111111", "alice", CAMERA);
    assert_eq!(a, b);
}

#[test]
fn distinct_senders_get_distinct_stream_ids() {
    let alice = derive_stream_id("11111111111111111111111111111111", "alice", CAMERA);
    let bob = derive_stream_id("11111111111111111111111111111111", "bob", CAMERA);
    assert_ne!(alice, bob);
}

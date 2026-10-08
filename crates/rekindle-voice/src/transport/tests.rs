//! Unit tests for [`super::VoiceTransport`] — extracted via `#[path]`
//! to keep `mod.rs` under the file-size ceiling.

use super::*;

/// Fails only for `dead_blob` — mirrors the frame-sender adapter's
/// wrapping of a veilid `NoConnection` into `VoiceError::Transport`.
struct SelectiveSender {
    dead_blob: Vec<u8>,
}

#[async_trait]
impl VoiceFrameSender for SelectiveSender {
    async fn send_voice_frame(&self, route_blob: &[u8], _data: Vec<u8>) -> Result<(), VoiceError> {
        if route_blob == self.dead_blob.as_slice() {
            Err(VoiceError::Transport(
                "app_message: No connection: could not get remote private route".into(),
            ))
        } else {
            Ok(())
        }
    }
}

fn test_transport(dead_blob: Vec<u8>) -> VoiceTransport {
    let mut t = VoiceTransport::new("chan".into());
    t.init(
        Arc::new(SelectiveSender { dead_blob }),
        vec![1, 2, 3],
        rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {})),
    );
    // build_packet_data requires a signing key.
    t.set_signing_key(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]));
    t
}

fn frame() -> OutboundFrame {
    OutboundFrame {
        sequence: 1,
        timestamp: 1,
        sframe: vec![0u8; 8],
        media_bytes: 8,
    }
}

#[test]
fn no_connection_classifier() {
    assert!(is_no_connection(&VoiceError::Transport(
        "app_message: No connection: could not get remote private route".into()
    )));
    assert!(!is_no_connection(&VoiceError::Transport(
        "app_message: Timeout".into()
    )));
    assert!(!is_no_connection(&VoiceError::NotConnected));
}

/// Each peer's driver delivers on its own: a dead route fails without
/// holding up the live one.
#[tokio::test]
async fn a_dead_route_does_not_hold_up_a_live_one() {
    let dead = vec![9u8, 9, 9];
    let live = vec![1u8, 1, 1];
    let mut t = test_transport(dead.clone());
    t.add_peer("dead_peer", &dead, None);
    t.add_peer("live_peer", &live, None);
    for _ in 0..3 {
        t.send(&frame()).unwrap();
    }
    let link = |k: &str| Arc::clone(&t.peers[k].link);
    let (dead_link, live_link) = (link("dead_peer"), link("live_peer"));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while live_link.send_counts().0 < 3 || dead_link.send_counts().1 < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both drivers ran");
    assert_eq!(live_link.send_counts(), (3, 0));
    assert_eq!(dead_link.send_counts(), (0, 3));
    assert_eq!(t.send_counts(), (3, 3));
}

#[test]
fn send_needs_a_peer() {
    let t = test_transport(Vec::new());
    assert!(matches!(t.send(&frame()), Err(VoiceError::NotConnected)));
}

#[tokio::test]
async fn removing_a_peer_stops_its_driver_and_forgets_its_allocation() {
    let mut t = test_transport(Vec::new());
    t.add_peer("p", &[1], None);
    let link = Arc::clone(&t.peers["p"].link);
    assert!(t.remove_peer("p"));
    assert!(link.is_stopped());
    assert!(t.allocator().route("p").is_none());
}

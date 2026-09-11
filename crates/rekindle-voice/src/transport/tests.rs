//! Unit tests for [`super::VoiceTransport`] — extracted via `#[path]`
//! to keep `mod.rs` under the file-size ceiling.

use super::*;
use crate::codec::EncodedFrame;

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
    t.init(Arc::new(SelectiveSender { dead_blob }), vec![1, 2, 3]);
    // build_packet_data requires a signing key.
    t.set_signing_key(ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]));
    t
}

fn frame() -> EncodedFrame {
    EncodedFrame {
        data: vec![0u8; 8],
        timestamp: 1,
        sequence: 1,
        mek_generation: 0,
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

#[tokio::test]
async fn broadcast_reports_dead_peer_but_send_survives_partial_failure() {
    let dead = vec![9u8, 9, 9];
    let live = vec![1u8, 1, 1];
    let mut t = test_transport(dead.clone());
    t.add_peer("dead_peer", &dead, None);
    t.add_peer("live_peer", &live, None);

    let errors = t.broadcast(&frame()).await;
    assert_eq!(errors.len(), 1, "only the dead peer should fail");
    assert_eq!(errors[0].0, "dead_peer");
    assert!(is_no_connection(&errors[0].1));

    // Partial failure must NOT surface from `send` — the live peer
    // keeps the call alive (the all-fail-only return contract).
    assert!(t.send(&frame()).await.is_ok());
}

#[tokio::test]
async fn send_fails_only_when_every_peer_fails() {
    let dead = vec![9u8, 9, 9];
    let mut t = test_transport(dead.clone());
    t.add_peer("dead_peer", &dead, None);
    assert!(t.send(&frame()).await.is_err());
}

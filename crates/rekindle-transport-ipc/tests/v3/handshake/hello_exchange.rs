use rekindle_transport_ipc::v3::session::handshake::{
    handshake_pair, HandshakeConfig,
};

fn default_config() -> HandshakeConfig {
    HandshakeConfig::default()
}

#[tokio::test]
async fn handshake_completes_over_socketpair() {
    let (dialler, listener) = handshake_pair(default_config(), default_config())
        .await
        .expect("handshake must complete");
    assert!(!dialler.handshake_hash.iter().all(|&b| b == 0));
    assert!(!listener.handshake_hash.iter().all(|&b| b == 0));
}

#[tokio::test]
async fn session_id_agreed() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.session_id, listener.session_id);
}

#[tokio::test]
async fn derived_keys_agree_envelope() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.keys.envelope_d2l, listener.keys.envelope_d2l);
    assert_eq!(dialler.keys.envelope_l2d, listener.keys.envelope_l2d);
}

#[tokio::test]
async fn derived_keys_agree_stream() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.keys.stream_d2l, listener.keys.stream_d2l);
    assert_eq!(dialler.keys.stream_l2d, listener.keys.stream_l2d);
}

#[tokio::test]
async fn derived_keys_agree_audit() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.keys.audit_d2l, listener.keys.audit_d2l);
    assert_eq!(dialler.keys.audit_l2d, listener.keys.audit_l2d);
}

#[tokio::test]
async fn derived_keys_agree_header() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.keys.header_d2l, listener.keys.header_d2l);
    assert_eq!(dialler.keys.header_l2d, listener.keys.header_l2d);
}

#[tokio::test]
async fn derived_keys_agree_handoff() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.keys.handoff, listener.keys.handoff);
}

#[tokio::test]
async fn peer_ids_exchanged() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.remote_peer_id, listener.local_peer_id);
    assert_eq!(listener.remote_peer_id, dialler.local_peer_id);
}

#[tokio::test]
async fn handshake_hash_matches_both_sides() {
    let (dialler, listener) = handshake_pair(default_config(), default_config()).await.unwrap();
    assert_eq!(dialler.handshake_hash, listener.handshake_hash);
}

#[tokio::test]
async fn emac_roundtrip_after_handshake() {
    use rekindle_transport_ipc::v3::codec::envelope::{build_envelope, parse_envelope, EnvelopeInfo};
    use rekindle_transport_ipc::v3::wire::lane::Lane;

    let (dialler, _listener) = handshake_pair(default_config(), default_config()).await.unwrap();

    let info = EnvelopeInfo {
        wire_version: rekindle_transport_ipc::v3::wire::constants::WIRE_VERSION,
        lane: Lane::Control,
        flags: 0,
        body_len: 100,
        session_seq: 1,
    };
    let envelope = build_envelope(&info, &dialler.keys.envelope_d2l);
    let parsed = parse_envelope(&envelope, &dialler.keys.envelope_d2l)
        .expect("EMAC must verify with the derived key");
    assert_eq!(parsed.session_seq, 1);
}

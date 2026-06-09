use rekindle_transport_ipc::v3::session::driver::FramePipeline;
use rekindle_transport_ipc::v3::codec::channel::ping;
use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::{ChannelKind, StreamKind, AuditKind};
use rekindle_transport_ipc::v3::wire::lane::Lane;

fn test_keys() -> rekindle_transport_ipc::v3::crypto::keys::DerivedKeys {
    derive_all_keys(&[0xAA; 32])
}

#[test]
fn control_frame_roundtrip() {
    let keys = test_keys();
    let mut pipeline = FramePipeline::new(&keys);

    let ping_payload = ping::PingPayload {
        ping_nonce: 0xDEAD,
        sender_epoch_ns: 123_456_789,
        last_seen_remote_seq: 0,
    };
    let ping_bytes = ping::encode(&ping_payload);

    let encoded = pipeline.build_control_packet(
        FrameClass::Channel as u8,
        ChannelKind::Ping as u8,
        &ping_bytes,
    );
    assert_eq!(encoded.session_seq, 0);

    let frame = pipeline.receive_packet(&encoded).expect("roundtrip must succeed");
    assert_eq!(frame.lane, Lane::Control);
    assert_eq!(frame.session_seq, 0);

    let decoded = ping::decode(&frame.payload[2..]).expect("ping decode");
    assert_eq!(decoded.ping_nonce, 0xDEAD);
    assert_eq!(decoded.sender_epoch_ns, 123_456_789);
}

#[test]
fn data_frame_roundtrip() {
    let keys = test_keys();
    let mut pipeline = FramePipeline::new(&keys);

    let payload = b"hello stream world";

    let encoded = pipeline.build_data_packet(
        StreamKind::Payload,
        0, // stream_id
        0, // chunk_index
        payload,
    );

    let frame = pipeline.receive_packet(&encoded).expect("roundtrip must succeed");
    assert_eq!(frame.lane, Lane::Data);
    assert_eq!(frame.stream_id, Some(0));
    assert_eq!(frame.payload, payload);
}

#[test]
fn multiple_frames_sequential_session_seq() {
    let keys = test_keys();
    let mut pipeline = FramePipeline::new(&keys);

    let ping_bytes = ping::encode(&ping::PingPayload {
        ping_nonce: 1,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });

    let p0 = pipeline.build_control_packet(
        FrameClass::Channel as u8,
        ChannelKind::Ping as u8,
        &ping_bytes,
    );
    let p1 = pipeline.build_control_packet(
        FrameClass::Channel as u8,
        ChannelKind::Ping as u8,
        &ping_bytes,
    );
    let p2 = pipeline.build_control_packet(
        FrameClass::Channel as u8,
        ChannelKind::Ping as u8,
        &ping_bytes,
    );

    assert_eq!(p0.session_seq, 0);
    assert_eq!(p1.session_seq, 1);
    assert_eq!(p2.session_seq, 2);

    let f0 = pipeline.receive_packet(&p0).unwrap();
    let f1 = pipeline.receive_packet(&p1).unwrap();
    let f2 = pipeline.receive_packet(&p2).unwrap();

    assert_eq!(f0.session_seq, 0);
    assert_eq!(f1.session_seq, 1);
    assert_eq!(f2.session_seq, 2);
}

#[test]
fn interleaved_lanes() {
    let keys = test_keys();
    let mut pipeline = FramePipeline::new(&keys);

    let ping_bytes = ping::encode(&ping::PingPayload {
        ping_nonce: 1,
        sender_epoch_ns: 0,
        last_seen_remote_seq: 0,
    });

    let control = pipeline.build_control_packet(
        FrameClass::Channel as u8,
        ChannelKind::Ping as u8,
        &ping_bytes,
    );

    let data = pipeline.build_data_packet(
        StreamKind::Payload,
        3,
        0,
        b"stream data",
    );

    let audit = pipeline.build_audit_packet(
        AuditKind::Checkpoint,
        &[0u8; 96],
    );

    let fc = pipeline.receive_packet(&control).unwrap();
    let fd = pipeline.receive_packet(&data).unwrap();
    let fa = pipeline.receive_packet(&audit).unwrap();

    assert_eq!(fc.lane, Lane::Control);
    assert_eq!(fd.lane, Lane::Data);
    assert_eq!(fa.lane, Lane::Audit);

    assert_eq!(fc.session_seq, 0);
    assert_eq!(fd.session_seq, 1);
    assert_eq!(fa.session_seq, 2);
}

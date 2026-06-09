use rekindle_transport_ipc::v3::handoff::sidechannel::{SideChannel, FdTag, SideChannelError};
use rekindle_transport_ipc::v3::handoff::memfd::{write_and_seal, map_readonly};

#[test]
fn create_pair() {
    let (a, b) = SideChannel::create_pair().expect("create_pair failed");
    assert!(a.fd() >= 0);
    assert!(b.fd() >= 0);
    assert_ne!(a.fd(), b.fd());
}

#[test]
fn send_and_recv_tagged_fd() {
    let (sender, receiver) = SideChannel::create_pair().unwrap();
    let data = vec![0xAB; 128];
    let sealed = write_and_seal("test", &data).unwrap();

    let tag = FdTag {
        stream_id: 7,
        flags: 0x01,
        payload_size: 128,
        payload_id: 0,
        fd: sealed.fd(),
    };
    sender.send_tagged_fd(&tag).expect("send failed");
    let received = receiver.recv_tagged_fd().expect("recv failed");

    assert_eq!(received.stream_id, 7);
    assert_eq!(received.flags, 0x01);
    assert_eq!(received.payload_size, 128);
    assert!(received.fd >= 0);
}

#[test]
fn tag_preserves_stream_id() {
    let (sender, receiver) = SideChannel::create_pair().unwrap();
    let sealed = write_and_seal("test", &[0; 10]).unwrap();
    sender.send_tagged_fd(&FdTag { stream_id: 42, flags: 0, payload_size: 10, payload_id: 0, fd: sealed.fd() }).unwrap();
    let received = receiver.recv_tagged_fd().unwrap();
    assert_eq!(received.stream_id, 42);
}

#[test]
fn tag_preserves_payload_size() {
    let (sender, receiver) = SideChannel::create_pair().unwrap();
    let sealed = write_and_seal("test", &[0; 10]).unwrap();
    sender.send_tagged_fd(&FdTag { stream_id: 0, flags: 0, payload_size: 1_048_576, payload_id: 0, fd: sealed.fd() }).unwrap();
    let received = receiver.recv_tagged_fd().unwrap();
    assert_eq!(received.payload_size, 1_048_576);
}

#[test]
fn tag_preserves_flags() {
    let (sender, receiver) = SideChannel::create_pair().unwrap();
    let sealed = write_and_seal("test", &[0; 10]).unwrap();
    sender.send_tagged_fd(&FdTag { stream_id: 0, flags: 0x01, payload_size: 10, payload_id: 0, fd: sealed.fd() }).unwrap();
    let received = receiver.recv_tagged_fd().unwrap();
    assert_eq!(received.flags, 0x01);
}

#[test]
fn multiple_fds_in_sequence() {
    let (sender, receiver) = SideChannel::create_pair().unwrap();
    for i in 0..5u8 {
        let sealed = write_and_seal("test", &[i; 16]).unwrap();
        sender.send_tagged_fd(&FdTag { stream_id: i, flags: 0, payload_size: 16, payload_id: 0, fd: sealed.fd() }).unwrap();
    }
    for i in 0..5u8 {
        let received = receiver.recv_tagged_fd().unwrap();
        assert_eq!(received.stream_id, i);
    }
}

#[test]
fn fd_passing_preserves_memfd_content() {
    let (sender_sc, receiver_sc) = SideChannel::create_pair().unwrap();
    let data = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let sealed = write_and_seal("test", &data).unwrap();

    sender_sc.send_tagged_fd(&FdTag { stream_id: 0, flags: 0x01, payload_size: 4, payload_id: 0, fd: sealed.fd() }).unwrap();
    let received = receiver_sc.recv_tagged_fd().unwrap();

    let mapping = map_readonly(received.fd, received.payload_size).expect("map failed on received fd");
    assert_eq!(mapping.as_slice(), &data);
}

#[test]
fn recv_on_empty_returns_wouldblock() {
    let (_sender, receiver) = SideChannel::create_pair().unwrap();
    let result = receiver.recv_tagged_fd();
    match result {
        Err(SideChannelError::WouldBlock) => {}
        other => panic!("Expected WouldBlock on empty channel, got {other:?}"),
    }
}

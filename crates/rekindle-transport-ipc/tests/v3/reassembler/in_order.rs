use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
    let digest = *blake3::hash(data).as_bytes();
    r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
}

#[test]
fn single_chunk_delivered() {
    let mut r = Reassembler::new(256);
    let delivered = ins(&mut r, 0, b"hello");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].0, 0);
    assert_eq!(&delivered[0].1[..], b"hello");
}

#[test]
fn three_chunks_in_order() {
    let mut r = Reassembler::new(256);
    let d0 = ins(&mut r, 0, b"aaa");
    let d1 = ins(&mut r, 1, b"bbb");
    let d2 = ins(&mut r, 2, b"ccc");
    assert_eq!(d0.len(), 1);
    assert_eq!(d1.len(), 1);
    assert_eq!(d2.len(), 1);
    assert_eq!(&d0[0].1[..], b"aaa");
    assert_eq!(&d1[0].1[..], b"bbb");
    assert_eq!(&d2[0].1[..], b"ccc");
}

#[test]
fn large_transfer_100_chunks() {
    let mut r = Reassembler::new(256);
    for i in 0u32..100 {
        let chunk = vec![i as u8; 64];
        let delivered = ins(&mut r, i, &chunk);
        assert_eq!(delivered.len(), 1, "chunk {i} must deliver immediately");
    }
    assert_eq!(r.next_expected(), 100);
}

#[test]
fn delivery_callback_receives_correct_bytes() {
    let mut r = Reassembler::new(256);
    let payload = vec![0xAA; 1024];
    let delivered = ins(&mut r, 0, &payload);
    assert_eq!(delivered[0].0, 0);
    assert_eq!(&delivered[0].1[..], &payload[..]);
}

#[test]
fn next_expected_starts_at_zero() {
    let r = Reassembler::new(256);
    assert_eq!(r.next_expected(), 0);
}

#[test]
fn next_expected_advances_with_delivery() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &[1]);
    assert_eq!(r.next_expected(), 1);
    ins(&mut r, 1, &[2]);
    assert_eq!(r.next_expected(), 2);
}

use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
    let digest = *blake3::hash(data).as_bytes();
    r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
}

#[test]
fn chunk_1_before_chunk_0() {
    let mut r = Reassembler::new(256);
    let d1 = ins(&mut r, 1, b"bbb");
    assert!(d1.is_empty(), "chunk 1 must be buffered");

    let d0 = ins(&mut r, 0, b"aaa");
    assert_eq!(d0.len(), 2, "chunk 0 must flush both 0 and 1");
    assert_eq!(d0[0].0, 0);
    assert_eq!(&d0[0].1[..], b"aaa");
    assert_eq!(d0[1].0, 1);
    assert_eq!(&d0[1].1[..], b"bbb");
}

#[test]
fn reverse_order_delivery() {
    let mut r = Reassembler::new(256);
    for i in (0..5u32).rev() {
        ins(&mut r, i, &[i as u8]);
    }
    assert_eq!(r.next_expected(), 5);
}

#[test]
fn gap_in_middle() {
    let mut r = Reassembler::new(256);
    let d0 = ins(&mut r, 0, b"a");
    let d1 = ins(&mut r, 1, b"b");
    assert_eq!(d0.len(), 1);
    assert_eq!(d1.len(), 1);

    let d3 = ins(&mut r, 3, b"d");
    let d4 = ins(&mut r, 4, b"e");
    assert!(d3.is_empty(), "chunk 3 buffered -- gap at 2");
    assert!(d4.is_empty(), "chunk 4 buffered -- gap at 2");

    let d2 = ins(&mut r, 2, b"c");
    assert_eq!(d2.len(), 3, "filling gap at 2 must flush 2, 3, 4");
    assert_eq!(d2[0].0, 2);
    assert_eq!(&d2[0].1[..], b"c");
    assert_eq!(d2[1].0, 3);
    assert_eq!(&d2[1].1[..], b"d");
    assert_eq!(d2[2].0, 4);
    assert_eq!(&d2[2].1[..], b"e");
}

#[test]
fn duplicate_chunk_ignored() {
    let mut r = Reassembler::new(256);
    let d0 = ins(&mut r, 0, b"first");
    assert_eq!(d0.len(), 1);

    let dup = ins(&mut r, 0, b"second");
    assert!(dup.is_empty(), "duplicate chunk_index must be silently dropped");
    assert_eq!(r.next_expected(), 1);
}

#[test]
fn many_gaps_resolved() {
    let mut r = Reassembler::new(256);

    let order: Vec<u32> = {
        let mut v: Vec<u32> = (0..50).collect();
        for i in (0..50).step_by(2) {
            if i + 1 < 50 {
                v.swap(i, i + 1);
            }
        }
        v[25..50].reverse();
        v
    };

    for &idx in &order {
        ins(&mut r, idx, &[idx as u8]);
    }

    assert_eq!(r.next_expected(), 50, "all 50 chunks must have been delivered");
    assert_eq!(r.buffered_count(), 0, "no chunks should remain buffered");
}

#[test]
fn out_of_order_buffered_count() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 5, &[5]);
    ins(&mut r, 3, &[3]);
    ins(&mut r, 7, &[7]);
    assert_eq!(r.buffered_count(), 3);
    assert_eq!(r.next_expected(), 0);
}

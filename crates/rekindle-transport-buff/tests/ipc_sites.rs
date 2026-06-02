#![cfg(not(loom))]
//! Six IPC call-site shape tests.
//!
//! Proves that rekindle-transport-buff's primitives can express every
//! buffer/reorder/pool/credit pattern used at the six sites in
//! rekindle-transport-ipc, without depending on any IPC-specific type.
//!
//! Each test instantiates the primitive shape the IPC site requires
//! and exercises the lifecycle that site follows. This is the proof
//! the extraction covers every site enumerated in RTB-SPEC-001 Part 1.

use rekindle_transport_buff::reorder::ReorderRing;
use rekindle_transport_buff::pool::SlabPool;
use rekindle_transport_buff::credit::CreditGuard;
use rekindle_transport_buff::dispatch::DispatchQueue;
use rekindle_transport_buff::resequencer::Resequencer;
use rekindle_transport_buff::traits::{
    ImmediateReturn, NoOpLifecycle, SpinWake,
};

// ---------------------------------------------------------------------------
// Site 1 — Send disassembly staging (SlabPool → DispatchQueue)
// ---------------------------------------------------------------------------

#[test]
fn site1_send_disassembly_staging() {
    // Acquire a slab from the pool, write chunk data, push to dispatch queue.
    let pool = SlabPool::<Vec<u8>>::new(8, ImmediateReturn, NoOpLifecycle);
    let queue = DispatchQueue::<Vec<u8>>::new(16, SpinWake);

    for seq in 0..8u64 {
        let mut guard = pool.try_acquire().unwrap();
        // Simulate writing chunk data.
        guard.extend_from_slice(&seq.to_le_bytes());
        let data = guard.clone();
        pool.release(guard);
        pool.reclaim();

        queue.try_push(seq, data).unwrap();
    }

    // Workers consume.
    for expected_seq in 0..8u64 {
        let (seq, data) = queue.pop().unwrap();
        assert_eq!(seq, expected_seq);
        let val = u64::from_le_bytes(data[..8].try_into().unwrap());
        assert_eq!(val, expected_seq);
    }
    assert!(queue.pop().is_none());
}

// ---------------------------------------------------------------------------
// Site 2 — Send crypto-dispatch queue (DispatchQueue, bounded MPMC)
// ---------------------------------------------------------------------------

#[test]
fn site2_send_crypto_dispatch() {
    // Bounded MPMC queue from control loop to rayon encrypt pool.
    let queue = DispatchQueue::<(u64, Vec<u8>)>::new(64, SpinWake);

    // Simulate: control loop pushes seal jobs.
    for i in 0..64u64 {
        let nonce = i; // assigned before dispatch (invariant 4)
        let plaintext = vec![i as u8; 32];
        queue.try_push(i, (nonce, plaintext)).unwrap();
    }

    // Simulate: workers pop and "seal".
    let mut sealed = Vec::new();
    while let Some((seq, (nonce, data))) = queue.pop() {
        // In production: cipher.seal_in_place(&nonce, &aad, &mut data)
        sealed.push((seq, nonce, data.len()));
    }

    assert_eq!(sealed.len(), 64);
    // Workers may pop in any order — but all items are consumed.
    assert!(queue.is_empty());
}

// ---------------------------------------------------------------------------
// Site 3 — Send reorder-after-crypto (ReorderRing, MP-fill / SC-drain)
// ---------------------------------------------------------------------------

#[test]
fn site3_send_reorder_after_crypto() {
    // Multiple encrypt workers publish sealed chunks out of order;
    // the writer drains the contiguous prefix in order.
    let ring = ReorderRing::<Vec<u8>>::new(64);

    // Simulate: workers complete in reverse order.
    for seq in (0..32u64).rev() {
        let sealed = vec![seq as u8; 16]; // "sealed" chunk
        ring.publish(seq, sealed).unwrap();
    }

    // Writer: drain contiguous prefix → all 32 in order.
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, data| {
        assert_eq!(data[0], seq as u8);
        delivered.push(seq);
    });
    assert_eq!(delivered.len(), 32);
    for (i, &seq) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
    }
}

// ---------------------------------------------------------------------------
// Site 4 — Receive decrypt-dispatch queue (DispatchQueue, bounded MPMC)
// ---------------------------------------------------------------------------

#[test]
fn site4_recv_decrypt_dispatch() {
    // Same shape as site 2 but for inbound ciphertext → rayon decrypt pool.
    // The bound here interacts with CreditGuard (site 6).
    let guard = CreditGuard::new(1024);
    let queue = DispatchQueue::<Vec<u8>>::new(16, SpinWake);

    for i in 0..16u64 {
        let chunk_len = 64u64;
        // Reserve memory first.
        assert!(guard.try_reserve(chunk_len));
        // Then dispatch to decrypt pool.
        let ciphertext = vec![i as u8; chunk_len as usize];
        queue.try_push(i, ciphertext).unwrap();
    }

    // Workers decrypt.
    while let Some((seq, data)) = queue.pop() {
        assert_eq!(data.len(), 64);
        // On delivery: release credit.
        guard.release(64);
        let _ = seq;
    }

    assert_eq!(guard.inflight(), 0);
}

// ---------------------------------------------------------------------------
// Site 5 — Receive reassembly (ReorderRing + SlabPool, MP-fill / SC-drain)
//           This is the ~50 MB knee site — the canonical ReorderRing use.
// ---------------------------------------------------------------------------

#[test]
fn site5_recv_reassembly_reorder() {
    // Verified plaintext chunks arrive out of order; the ring reorders
    // for in-order delivery. The SlabPool is the arena backing.
    let ring = ReorderRing::<Vec<u8>>::new(256);

    // Simulate: chunks arrive in scrambled order.
    let arrival_order: Vec<u64> = {
        let mut v: Vec<u64> = (0..128).collect();
        // Deterministic scramble: reverse pairs.
        for i in (0..v.len()).step_by(2) {
            if i + 1 < v.len() {
                v.swap(i, i + 1);
            }
        }
        v
    };

    for &seq in &arrival_order {
        let plaintext = vec![seq as u8; 32];
        ring.publish(seq, plaintext).unwrap();
    }

    // Single consumer: drain in order.
    let mut delivered = Vec::new();
    ring.drain_contiguous(|seq, data| {
        assert_eq!(data[0], seq as u8);
        delivered.push(seq);
    });

    assert_eq!(delivered.len(), 128);
    for (i, &seq) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64, "out-of-order delivery at index {i}");
    }
}

// ---------------------------------------------------------------------------
// Site 6 — Send FixedBuf pool + recv GlobalMemoryGuard
//           (SlabPool with ReturnGate + CreditGuard)
// ---------------------------------------------------------------------------

#[test]
fn site6_send_pool_closed_loop() {
    // Closed-loop: send pool bounds memory by slab count.
    let pool = SlabPool::<Vec<u8>>::new(4, ImmediateReturn, NoOpLifecycle);

    // Exhaust all 4 slabs.
    let guards: Vec<_> = (0..4)
        .map(|_| pool.try_acquire().unwrap())
        .collect();
    // Pool exhausted — backpressure.
    assert!(pool.try_acquire().is_none());

    // Release and reclaim.
    for g in guards {
        pool.release(g);
    }
    pool.reclaim();
    assert_eq!(pool.available(), 4);
}

#[test]
fn site6_recv_credit_guard_open_loop() {
    // Open-loop: credit guard bounds recv memory by byte ceiling.
    let guard = CreditGuard::new(1024);

    // Admit chunks until ceiling.
    assert!(guard.try_reserve(512));
    assert!(guard.try_reserve(512));
    // At ceiling — shed.
    assert!(!guard.try_reserve(1));

    // Deliver → release.
    guard.release(512);
    assert_eq!(guard.headroom(), 512);

    // Can admit again.
    assert!(guard.try_reserve(512));
    guard.release(1024);
    assert_eq!(guard.inflight(), 0);
}

// ---------------------------------------------------------------------------
// Composed: Resequencer covers sites 2+3 and 4+5 as a single unit
// ---------------------------------------------------------------------------

#[test]
fn composed_resequencer_covers_dispatch_and_reorder() {
    let rs = Resequencer::<Vec<u8>, Vec<u8>>::new(16, 16, SpinWake, SpinWake);

    // Submit 10 items with seq assigned at submit time.
    for i in 0..10u64 {
        rs.submit(i, vec![i as u8; 8]).unwrap();
    }

    // Simulate 4 workers processing out of order.
    let mut guards: Vec<_> = (0..10)
        .map(|_| rs.take_work().unwrap())
        .collect();

    // Complete in reverse. pop() gives ownership — complete() consumes self.
    while let Some(mut guard) = guards.pop() {
        let input = guard.take_input().unwrap();
        let output: Vec<u8> = input.iter().map(|b| b + 1).collect();
        guard.complete(output).unwrap();
    }

    // Drain: all 10 in order.
    let mut delivered = Vec::new();
    rs.drain(|seq, result| {
        let data = result.unwrap();
        delivered.push((seq, data[0]));
    });
    assert_eq!(delivered.len(), 10);
    for (i, &(seq, first_byte)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
        assert_eq!(first_byte, i as u8 + 1);
    }
}

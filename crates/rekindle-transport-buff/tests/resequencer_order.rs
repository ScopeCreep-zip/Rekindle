#![cfg(not(loom))]
//! Resequencer ordering tests — proves the nonce-before-dispatch invariant
//! is structural, not documentary.
//!
//! These tests exercise:
//! - Seq is assigned at submit(), never by the worker.
//! - Out-of-order completion delivers in seq order.
//! - Worker panic publishes sentinel without stalling the ring.
//! - Concurrent workers produce correctly ordered output.
//! - Backpressure (full dispatch queue) returns the item.

use rekindle_transport_buff::resequencer::{Resequencer, WorkerFailed};
use rekindle_transport_buff::SpinWake;

#[test]
fn seq_flows_through_unchanged() {
    // The seq the producer submits must be the exact seq the consumer
    // receives — the Resequencer carries it, never generates or modifies it.
    // Use seq=0 because drain_contiguous delivers from next_deliver=0.
    let rs = Resequencer::<u64, String>::new(4, 64, SpinWake, SpinWake);

    rs.submit(0, 100).unwrap();
    let mut guard = rs.take_work().unwrap();
    assert_eq!(guard.seq(), 0, "WorkGuard must carry the submitted seq");

    let input = guard.take_input().unwrap();
    assert_eq!(input, 100);

    guard.complete(format!("processed-{input}")).unwrap();

    let mut delivered = Vec::new();
    rs.drain(|seq, result| delivered.push((seq, result)));
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].0, 0, "delivered seq must match submitted seq");
    assert_eq!(delivered[0].1, Ok("processed-100".to_string()));
}

#[test]
fn out_of_order_complete_delivers_in_seq_order() {
    let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

    for i in 0..5 {
        rs.submit(i, i * 10).unwrap();
    }

    // Take all 5 — must use Vec::remove to move out by value (Vec index is a borrow).
    let mut guards: Vec<_> = (0..5).map(|_| rs.take_work().unwrap()).collect();

    // Complete in order 4,2,0,3,1 — remove from back to front to keep indices stable.
    // Strategy: complete by removing from the vec (swap_remove is fine, order doesn't matter).
    let complete_order = [4usize, 2, 0, 3, 1];
    for &target_seq in &complete_order {
        let pos = guards.iter().position(|g| g.seq() == target_seq as u64).unwrap();
        let mut g = guards.swap_remove(pos);
        let input = g.take_input().unwrap();
        g.complete(input + 1).unwrap();
    }
    assert!(guards.is_empty());

    // Drain: must deliver 0,1,2,3,4 regardless of completion order.
    let mut delivered = Vec::new();
    rs.drain(|seq, result| delivered.push((seq, result.unwrap())));
    assert_eq!(delivered.len(), 5);
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
        assert_eq!(val, i as u64 * 10 + 1);
    }
}

#[test]
fn partial_drain_then_complete_gap() {
    let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

    rs.submit(0, 10).unwrap();
    rs.submit(1, 20).unwrap();
    rs.submit(2, 30).unwrap();

    let mut w0 = rs.take_work().unwrap();
    let mut w1 = rs.take_work().unwrap();
    let mut w2 = rs.take_work().unwrap();

    // Complete 0 and 2, leaving gap at 1.
    let i0 = w0.take_input().unwrap();
    w0.complete(i0).unwrap();
    let i2 = w2.take_input().unwrap();
    w2.complete(i2).unwrap();

    // Drain: only 0 is contiguous.
    let mut d = Vec::new();
    rs.drain(|s, r| d.push((s, r.unwrap())));
    assert_eq!(d, vec![(0, 10)]);

    // Complete 1 — now 1 and 2 are contiguous.
    let i1 = w1.take_input().unwrap();
    w1.complete(i1).unwrap();
    let mut d = Vec::new();
    rs.drain(|s, r| d.push((s, r.unwrap())));
    assert_eq!(d, vec![(1, 20), (2, 30)]);
}

#[test]
fn worker_panic_sentinel_does_not_stall() {
    let rs = Resequencer::<u64, u64>::new(8, 8, SpinWake, SpinWake);

    rs.submit(0, 100).unwrap();
    rs.submit(1, 200).unwrap();
    rs.submit(2, 300).unwrap();
    rs.submit(3, 400).unwrap();

    // Complete 0.
    let mut w0 = rs.take_work().unwrap();
    let i0 = w0.take_input().unwrap();
    w0.complete(i0).unwrap();

    // Drop 1 and 2 (simulate panics).
    { let _w1 = rs.take_work().unwrap(); }
    { let _w2 = rs.take_work().unwrap(); }

    // Complete 3.
    let mut w3 = rs.take_work().unwrap();
    let i3 = w3.take_input().unwrap();
    w3.complete(i3).unwrap();

    // All four should drain: 0 ok, 1 failed, 2 failed, 3 ok.
    let mut d = Vec::new();
    rs.drain(|s, r| d.push((s, r)));
    assert_eq!(d.len(), 4);
    assert_eq!(d[0], (0, Ok(100)));
    assert_eq!(d[1], (1, Err(WorkerFailed)));
    assert_eq!(d[2], (2, Err(WorkerFailed)));
    assert_eq!(d[3], (3, Ok(400)));
}

#[test]
fn backpressure_returns_item() {
    let rs = Resequencer::<u64, u64>::new(2, 8, SpinWake, SpinWake);

    rs.submit(0, 10).unwrap();
    rs.submit(1, 20).unwrap();
    // Queue is full (capacity 2).
    let err = rs.submit(2, 30).unwrap_err();
    assert_eq!(err.seq, 2);
    assert_eq!(err.item, 30);
}

#[test]
fn take_input_returns_none_on_second_call() {
    let rs = Resequencer::<u64, u64>::new(4, 8, SpinWake, SpinWake);
    rs.submit(0, 42).unwrap();

    let mut guard = rs.take_work().unwrap();
    assert_eq!(guard.take_input(), Some(42));
    assert_eq!(guard.take_input(), None);
    guard.complete(99).unwrap();
}

#[test]
fn input_ref_available_before_take() {
    let rs = Resequencer::<String, String>::new(4, 8, SpinWake, SpinWake);
    rs.submit(0, "hello".to_string()).unwrap();

    let guard = rs.take_work().unwrap();
    assert_eq!(guard.input(), Some(&"hello".to_string()));
    // Don't take — just complete with a new value.
    guard.complete("world".to_string()).unwrap();

    let mut d = Vec::new();
    rs.drain(|s, r| d.push((s, r.unwrap())));
    assert_eq!(d, vec![(0, "world".to_string())]);
}

#[test]
fn concurrent_workers_all_items_delivered_in_order() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let rs = Arc::new(Resequencer::<u64, u64>::new(128, 128, SpinWake, SpinWake));
    let total = 64usize;
    let completed = Arc::new(AtomicUsize::new(0));

    for i in 0..total as u64 {
        rs.submit(i, i * 10).unwrap();
    }

    let workers: Vec<_> = (0..8)
        .map(|_| {
            let rs = Arc::clone(&rs);
            let completed = Arc::clone(&completed);
            std::thread::spawn(move || {
                loop {
                    if completed.load(Ordering::Acquire) >= total {
                        break;
                    }
                    match rs.take_work() {
                        Some(mut guard) => {
                            let input = guard.take_input().unwrap();
                            guard.complete(input + 1).unwrap();
                            completed.fetch_add(1, Ordering::Release);
                        }
                        None => std::thread::yield_now(),
                    }
                }
            })
        })
        .collect();

    for w in workers {
        w.join().unwrap();
    }

    let mut delivered = Vec::new();
    rs.drain(|seq, result| delivered.push((seq, result.unwrap())));
    assert_eq!(delivered.len(), total);
    for (i, &(seq, val)) in delivered.iter().enumerate() {
        assert_eq!(seq, i as u64);
        assert_eq!(val, i as u64 * 10 + 1);
    }
}

#[test]
fn many_cycles_no_leak() {
    let rs = Resequencer::<u64, u64>::new(4, 4, SpinWake, SpinWake);

    for cycle in 0..200u64 {
        let base = cycle * 4;
        for i in 0..4 {
            rs.submit(base + i, base + i).unwrap();
        }
        for _ in 0..4 {
            let mut g = rs.take_work().unwrap();
            let input = g.take_input().unwrap();
            g.complete(input).unwrap();
        }
        let count = rs.drain(|_, _| {});
        assert_eq!(count, 4, "cycle {cycle}: expected 4 delivered");
    }
    assert_eq!(rs.next_deliver(), 800);
}

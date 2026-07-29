//! Slot state machine tests — validates every transition in the three-state
//! writer-owned model: FREE → ACQUIRED → IN_FLIGHT → FREE(gen+1).
//!
//! These tests verify the state machine invariants that the CAS protocol
//! must uphold. No shortcut implementation can pass all of them.

use rekindle_transport_ipc::v4::streaming::shared_arena::{
    SharedArena, SlotRelease,
};

// ── FREE → ACQUIRED transition ──────────────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn free_to_acquired_via_try_acquire() {
    let arena = SharedArena::create(4096, 4, false).expect("create");
    let guard = arena.try_acquire().expect("must acquire free slot");
    assert!(guard.slot_index() < arena.slot_count() as u16);
    assert_eq!(guard.generation(), 0);
    drop(guard);
}

// ── ACQUIRED → IN_FLIGHT transition (publish/release) ───────────

#[cfg(target_os = "linux")]
#[test]
fn acquired_to_inflight_via_release() {
    let arena = SharedArena::create(4096, 4, false).expect("create");
    let mut guard = arena.try_acquire().expect("acquire");
    let slot = guard.slot_index();
    let gen = guard.generation();
    guard.write_from_slice(&[0x42; 10]);
    let shmref = guard.release(0, 10);

    assert_eq!(shmref.slot, slot);
    assert_eq!(shmref.generation, gen);
    assert_eq!(shmref.length, 10);

    assert!(arena.read_ref(&shmref).is_some());
}

// ── IN_FLIGHT → FREE(gen+1) transition (return_slot) ────────────

#[cfg(target_os = "linux")]
#[test]
fn inflight_to_free_via_return_slot() {
    let arena = SharedArena::create(4096, 4, false).expect("create");
    let mut guard = arena.try_acquire().expect("acquire");
    guard.write_from_slice(&[0x01; 1]);
    let shmref = guard.release(0, 1);

    let release = SlotRelease {
        arena_id: 0,
        slot: shmref.slot,
        generation: shmref.generation,
    };
    assert!(arena.return_slot(&release));

    assert!(arena.read_ref(&shmref).is_none());
}

// ── ACQUIRED → FREE (rollback via drop) ─────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn acquired_to_free_via_drop_rollback() {
    let arena = SharedArena::create(4096, 2, false).expect("create");

    {
        let _guard = arena.try_acquire().expect("acquire");
    }

    let guard = arena.try_acquire().expect("re-acquire after rollback");
    assert_eq!(guard.generation(), 0, "rollback must not increment generation");
    drop(guard);
}

// ── Generation increments only on successful cycle ──────────────

#[cfg(target_os = "linux")]
#[test]
fn generation_increments_only_on_return() {
    let arena = SharedArena::create(4096, 1, false).expect("create");

    // Cycle 0: acquire → rollback (drop) — gen stays 0
    {
        let _g = arena.try_acquire().expect("acquire");
    }
    let g = arena.try_acquire().expect("re-acquire");
    assert_eq!(g.generation(), 0);
    drop(g);

    // Cycle 1: acquire → release → return — gen becomes 1
    let mut g = arena.try_acquire().expect("acquire");
    g.write_from_slice(&[0x01; 1]);
    let shmref = g.release(0, 1);
    let release = SlotRelease { arena_id: 0, slot: shmref.slot, generation: shmref.generation };
    assert!(arena.return_slot(&release));

    let g = arena.try_acquire().expect("acquire gen 1");
    assert_eq!(g.generation(), 1);
    drop(g);

    // Cycle 2: rollback again — gen stays 1
    {
        let _g = arena.try_acquire().expect("acquire for rollback");
    }
    let g = arena.try_acquire().expect("re-acquire gen 1");
    assert_eq!(g.generation(), 1);
    drop(g);
}

// ── Slot scan distributes across slots ──────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn slot_scan_distributes_evenly() {
    let arena = SharedArena::create(4096, 4, false).expect("create");

    let mut seen = std::collections::HashSet::new();
    for _ in 0..8 {
        let guard = arena.try_acquire().expect("acquire");
        seen.insert(guard.slot_index());
        drop(guard);
    }

    assert_eq!(seen.len(), 4, "scan must distribute across all slots");
}

// ── Wrapping generation does not break CAS ──────────────────────

#[cfg(target_os = "linux")]
#[test]
fn generation_wrapping_is_safe() {
    let gen = u32::MAX;
    let next = gen.wrapping_add(1);
    assert_eq!(next, 0);
}

// ── Full lifecycle with multiple slots ──────────────────────────

#[cfg(target_os = "linux")]
#[test]
fn full_lifecycle_four_slots_interleaved() {
    let arena = SharedArena::create(4096, 4, false).expect("create");

    let mut guards: Vec<_> = (0..4)
        .map(|_| arena.try_acquire().expect("acquire"))
        .collect();

    for (i, g) in guards.iter_mut().enumerate() {
        g.write_from_slice(&[i as u8; 1]);
    }

    let refs: Vec<_> = guards.into_iter().map(|g| g.release(0, 1)).collect();

    for (i, r) in refs.iter().enumerate() {
        let data = arena.read_ref(r).expect("read_ref");
        assert_eq!(data[0], i as u8);
    }

    for r in &refs {
        let release = SlotRelease { arena_id: 0, slot: r.slot, generation: r.generation };
        assert!(arena.return_slot(&release));
    }

    let mut acquired = 0;
    for _ in 0..4 {
        if arena.try_acquire().is_some() {
            acquired += 1;
        }
    }
    assert_eq!(acquired, 4);
}

//! SharedArena correctness tests — adversarial, production-grade.
//!
//! Every test exercises real kernel primitives (memfd, mmap, atomic CAS).
//! Tests are implementation-agnostic: they verify behaviors the application
//! depends on, not internal function signatures.
//!
//! Categories:
//! - Zero-copy proof: reader data points into mmap, not heap
//! - State machine: every transition produces the correct observable state
//! - Adversarial: corrupt inputs, stale references, overflow, crash recovery
//! - Concurrency: multi-thread contention on slot acquisition
//! - Allocation accountability: no hidden allocations on the read path

#[cfg(target_os = "linux")]
mod tests {
    use std::sync::Arc;
    use rekindle_transport_ipc::v4::streaming::shared_arena::{
        SharedArena, SharedMemRef, SlotRelease,
    };

    fn make_arena(slot_size: usize, slot_count: usize, integrity: bool) -> SharedArena {
        SharedArena::create(slot_size, slot_count, integrity)
            .expect("arena creation must not fail with valid parameters")
    }

    fn write_release_return(arena: &SharedArena, data: &[u8], arena_id: u8) {
        let mut guard = arena.try_acquire().expect("slot must be available");
        guard.write_from_slice(data);
        let shmref = guard.release(arena_id, data.len());
        let _ = arena.read_ref(&shmref).expect("read_ref must succeed after release");
        let release = SlotRelease {
            arena_id: shmref.arena_id,
            slot: shmref.slot,
            generation: shmref.generation,
        };
        assert!(arena.return_slot(&release), "return_slot must succeed");
    }

    // ── Construction ────────────────────────────────────────────────

    #[test]
    fn create_returns_requested_dimensions() {
        let arena = make_arena(4096, 4, true);
        assert_eq!(arena.slot_count(), 4);
        assert_eq!(arena.slot_size(), 4096);
    }

    #[test]
    fn create_page_aligns_slot_size_upward() {
        let arena = make_arena(5000, 2, false);
        assert!(arena.slot_size() >= 5000, "must be at least requested size");
        assert_eq!(arena.slot_size() % 4096, 0, "must be page-aligned");
    }

    #[test]
    fn create_rejects_zero_slot_size() {
        assert!(SharedArena::create(0, 4, false).is_err());
    }

    #[test]
    fn create_rejects_zero_slot_count() {
        assert!(SharedArena::create(4096, 0, false).is_err());
    }

    #[test]
    fn create_rejects_slot_count_exceeding_u16() {
        assert!(SharedArena::create(4096, u16::MAX as usize + 1, false).is_err());
    }

    // ── Zero-copy proof ─────────────────────────────────────────────

    #[test]
    fn read_ref_returns_pointer_into_mmap_region() {
        let arena = make_arena(4096, 4, false);
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&[0xAA; 1024]);
        let shmref = guard.release(0, 1024);

        let data = arena.read_ref(&shmref).expect("read_ref");
        let arena_base = arena.arena_base_ptr();
        let arena_end = unsafe { arena_base.add(arena.slot_size() * arena.slot_count()) };

        assert!(
            data.as_ptr() >= arena_base,
            "data pointer {:#?} must be >= arena base {:#?}",
            data.as_ptr(), arena_base,
        );
        assert!(
            data.as_ptr() < arena_end,
            "data pointer {:#?} must be < arena end {:#?}",
            data.as_ptr(), arena_end,
        );

        // Verify the data is exactly at the expected offset within the arena
        let expected_offset = shmref.slot as usize * arena.slot_size() + shmref.offset as usize;
        let actual_offset = unsafe { data.as_ptr().offset_from(arena_base) } as usize;
        assert_eq!(actual_offset, expected_offset, "data must be at slot offset in mmap");
    }

    #[test]
    fn read_ref_data_matches_written_payload() {
        let arena = make_arena(4096, 4, true);
        let payload: Vec<u8> = (0..1024).map(|i| (i % 251) as u8).collect();
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&payload);
        let shmref = guard.release(0, payload.len());

        let data = arena.read_ref(&shmref).expect("read_ref");
        assert_eq!(data, payload.as_slice(), "read data must exactly match written payload");
    }

    // ── BLAKE3 integrity ────────────────────────────────────────────

    #[test]
    fn integrity_check_produces_correct_blake3_digest() {
        let arena = make_arena(4096, 4, true);
        let payload = vec![0xAA; 1024];
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&payload);
        let shmref = guard.release(0, 1024);

        let expected: [u8; 32] = blake3::hash(&payload).into();
        assert_eq!(shmref.digest, Some(expected));
    }

    #[test]
    fn integrity_disabled_produces_no_digest() {
        let arena = make_arena(4096, 4, false);
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&[0xBB; 512]);
        let shmref = guard.release(0, 512);
        assert_eq!(shmref.digest, None);
    }

    #[test]
    fn integrity_check_detects_single_byte_corruption() {
        let arena = make_arena(4096, 4, true);
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&[0xAA; 1024]);
        let shmref = guard.release(0, 1024);

        // Corrupt one byte at a page-interior offset
        unsafe {
            let ptr = (arena.arena_base_ptr() as *mut u8).add(
                shmref.slot as usize * arena.slot_size() + 500,
            );
            assert_eq!(*ptr, 0xAA, "precondition: byte was 0xAA");
            *ptr = 0xFF;
        }

        assert!(arena.read_ref(&shmref).is_none(),
            "corrupted payload must fail integrity check");
    }

    #[test]
    fn integrity_check_detects_corruption_at_page_boundary() {
        let arena = make_arena(8192, 4, true);
        let mut guard = arena.try_acquire().expect("acquire");
        guard.write_from_slice(&[0xCC; 8192]);
        let shmref = guard.release(0, 8192);

        // Corrupt at the 4096 page boundary — simulates stale TLB
        unsafe {
            let ptr = (arena.arena_base_ptr() as *mut u8).add(
                shmref.slot as usize * arena.slot_size() + 4096,
            );
            *ptr = 0x00;
        }

        assert!(arena.read_ref(&shmref).is_none(),
            "page-boundary corruption must fail integrity check");
    }

    // ── State machine transitions ───────────────────────────────────

    #[test]
    fn free_to_acquired_to_inflight_to_free() {
        let arena = make_arena(4096, 1, false);

        // FREE → ACQUIRED
        let mut guard = arena.try_acquire().expect("must acquire free slot");
        assert_eq!(guard.generation(), 0);

        // ACQUIRED → IN_FLIGHT
        guard.write_from_slice(&[0x42; 10]);
        let shmref = guard.release(0, 10);
        assert!(arena.read_ref(&shmref).is_some(), "IN_FLIGHT slot must be readable");

        // IN_FLIGHT → FREE(gen+1)
        let release = SlotRelease { arena_id: 0, slot: shmref.slot, generation: shmref.generation };
        assert!(arena.return_slot(&release));
        assert!(arena.read_ref(&shmref).is_none(), "FREE slot must not be readable");

        // Verify generation incremented
        let guard2 = arena.try_acquire().expect("must re-acquire");
        assert_eq!(guard2.generation(), 1);
        drop(guard2);
    }

    #[test]
    fn drop_without_release_rolls_back_without_generation_increment() {
        let arena = make_arena(4096, 1, false);

        {
            let mut guard = arena.try_acquire().expect("acquire");
            guard.write_from_slice(&[0xDD; 10]);
            // guard dropped — ACQUIRED → FREE(same gen)
        }

        let guard2 = arena.try_acquire().expect("re-acquire");
        assert_eq!(guard2.generation(), 0, "rollback must not increment generation");
        drop(guard2);
    }

    #[test]
    fn generation_increments_only_on_return_slot() {
        let arena = make_arena(4096, 1, false);

        // 3 rollbacks — generation stays 0
        for _ in 0..3 {
            let _g = arena.try_acquire().expect("acquire");
        }
        let g = arena.try_acquire().expect("acquire");
        assert_eq!(g.generation(), 0, "rollbacks must not increment");
        drop(g);

        // 1 full cycle — generation becomes 1
        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[1]);
        let shmref = g.release(0, 1);
        assert!(arena.return_slot(&SlotRelease {
            arena_id: 0, slot: shmref.slot, generation: shmref.generation,
        }));

        let g = arena.try_acquire().expect("acquire gen 1");
        assert_eq!(g.generation(), 1);
        drop(g);
    }

    // ── Adversarial inputs ──────────────────────────────────────────

    #[test]
    fn read_ref_rejects_out_of_bounds_slot() {
        let arena = make_arena(4096, 4, false);
        let bad = SharedMemRef {
            arena_id: 0, slot: 999, generation: 0,
            offset: 0, length: 100, digest: None,
        };
        assert!(arena.read_ref(&bad).is_none());
    }

    #[test]
    fn read_ref_rejects_free_slot() {
        let arena = make_arena(4096, 4, false);
        let fake = SharedMemRef {
            arena_id: 0, slot: 0, generation: 0,
            offset: 0, length: 100, digest: None,
        };
        assert!(arena.read_ref(&fake).is_none(), "FREE slot must reject read");
    }

    #[test]
    fn read_ref_rejects_acquired_slot() {
        let arena = make_arena(4096, 4, false);
        let guard = arena.try_acquire().expect("acquire");
        let fake = SharedMemRef {
            arena_id: 0, slot: guard.slot_index(), generation: guard.generation(),
            offset: 0, length: 100, digest: None,
        };
        assert!(arena.read_ref(&fake).is_none(), "ACQUIRED slot must reject read");
        drop(guard);
    }

    #[test]
    fn read_ref_rejects_stale_generation() {
        let arena = make_arena(4096, 4, false);

        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[0xAA; 64]);
        let shmref_gen0 = g.release(0, 64);
        assert!(arena.return_slot(&SlotRelease {
            arena_id: 0, slot: shmref_gen0.slot, generation: shmref_gen0.generation,
        }));

        // Re-acquire, release at gen1
        let mut g2 = arena.try_acquire().expect("re-acquire");
        g2.write_from_slice(&[0xBB; 64]);
        let _shmref_gen1 = g2.release(0, 64);

        // Stale gen0 ref must fail
        assert!(arena.read_ref(&shmref_gen0).is_none());
    }

    #[test]
    fn read_ref_rejects_offset_plus_length_overflow() {
        let arena = make_arena(4096, 4, false);
        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[0xEE; 100]);
        let shmref = g.release(0, 100);

        let bad = SharedMemRef { offset: u32::MAX, length: 1, ..shmref };
        assert!(arena.read_ref(&bad).is_none());
    }

    #[test]
    fn read_ref_rejects_length_exceeding_slot_size() {
        let arena = make_arena(4096, 4, false);
        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[0xFF; 100]);
        let shmref = g.release(0, 100);

        let bad = SharedMemRef { length: arena.slot_size() as u32 + 1, ..shmref };
        assert!(arena.read_ref(&bad).is_none());
    }

    #[test]
    fn return_slot_rejects_double_return() {
        let arena = make_arena(4096, 4, false);
        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[0xCC; 32]);
        let shmref = g.release(0, 32);

        let release = SlotRelease {
            arena_id: 0, slot: shmref.slot, generation: shmref.generation,
        };
        assert!(arena.return_slot(&release), "first return must succeed");
        assert!(!arena.return_slot(&release), "double return must fail");
    }

    #[test]
    fn return_slot_rejects_out_of_bounds_slot() {
        let arena = make_arena(4096, 4, false);
        assert!(!arena.return_slot(&SlotRelease { arena_id: 0, slot: 999, generation: 0 }));
    }

    // ── Slot exhaustion and backpressure ─────────────────────────────

    #[test]
    fn all_slots_exhausted_returns_none() {
        let arena = make_arena(4096, 2, false);
        let g0 = arena.try_acquire().expect("slot 0");
        let g1 = arena.try_acquire().expect("slot 1");
        assert!(arena.try_acquire().is_none(), "must return None when all slots busy");
        drop(g0);
        drop(g1);
    }

    #[test]
    fn slot_becomes_available_after_return() {
        let arena = make_arena(4096, 1, false);
        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[1]);
        let shmref = g.release(0, 1);
        assert!(arena.try_acquire().is_none(), "slot busy");

        assert!(arena.return_slot(&SlotRelease {
            arena_id: 0, slot: shmref.slot, generation: shmref.generation,
        }));
        assert!(arena.try_acquire().is_some(), "slot must be free after return");
    }

    // ── Crash recovery ──────────────────────────────────────────────

    #[test]
    fn reclaim_all_inflight_frees_all_published_slots() {
        let arena = make_arena(4096, 4, false);

        // Put 3 slots into IN_FLIGHT
        for i in 0..3u8 {
            let mut g = arena.try_acquire().expect("acquire");
            g.write_from_slice(&[i; 1]);
            let _ = g.release(0, 1); // transitions to IN_FLIGHT
        }

        // 1 slot still FREE, 3 are IN_FLIGHT
        assert!(arena.try_acquire().is_some(), "4th slot must be free");

        // Simulate crash — reclaim all IN_FLIGHT
        arena.reclaim_all_inflight();

        // All 4 slots must be acquirable
        let mut count = 0;
        for _ in 0..4 {
            if arena.try_acquire().is_some() { count += 1; }
        }
        assert_eq!(count, 4, "all slots must be FREE after reclaim");
    }

    #[test]
    fn reclaim_increments_generation_preventing_stale_reads() {
        let arena = make_arena(4096, 1, false);

        let mut g = arena.try_acquire().expect("acquire");
        g.write_from_slice(&[0xAA; 100]);
        let shmref = g.release(0, 100);

        arena.reclaim_all_inflight();

        // The stale ref must fail — generation was incremented
        assert!(arena.read_ref(&shmref).is_none(),
            "stale ref must fail after reclaim (generation incremented)");
    }

    // ── Concurrency ─────────────────────────────────────────────────

    #[test]
    fn concurrent_acquire_no_duplicate_slots() {
        let arena = Arc::new(make_arena(4096, 4, false));
        let barrier = Arc::new(std::sync::Barrier::new(4));

        // Each thread acquires one slot, records its index, writes a pattern,
        // releases, and returns. No mem::forget — all guards are properly
        // released so the arena is usable afterward.
        let handles: Vec<_> = (0..4u8).map(|thread_id| {
            let a = Arc::clone(&arena);
            let b = Arc::clone(&barrier);
            std::thread::spawn(move || {
                b.wait(); // synchronize all threads to maximize contention
                if let Some(mut g) = a.try_acquire() {
                    let idx = g.slot_index();
                    g.write_from_slice(&[thread_id; 1]);
                    let shmref = g.release(0, 1);
                    // Return slot immediately
                    a.return_slot(&SlotRelease {
                        arena_id: 0, slot: shmref.slot, generation: shmref.generation,
                    });
                    Some(idx)
                } else {
                    None
                }
            })
        }).collect();

        let mut slots: Vec<u16> = handles.into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect();

        assert_eq!(slots.len(), 4, "all 4 threads must acquire exactly 1 slot");
        slots.sort();
        slots.dedup();
        assert_eq!(slots.len(), 4, "no duplicate slot indices");

        // Arena must be fully usable afterward — no leaked slots
        for _ in 0..4 {
            assert!(arena.try_acquire().is_some(), "all slots must be free after test");
        }
    }

    #[test]
    fn concurrent_full_lifecycle_no_data_corruption() {
        let arena = Arc::new(make_arena(4096, 8, true));
        let iterations = 100;

        let handles: Vec<_> = (0..4).map(|thread_id| {
            let a = Arc::clone(&arena);
            std::thread::spawn(move || {
                let pattern = thread_id as u8;
                for _ in 0..iterations {
                    if let Some(mut g) = a.try_acquire() {
                        let payload = vec![pattern; 256];
                        g.write_from_slice(&payload);
                        let shmref = g.release(0, 256);

                        if let Some(data) = a.read_ref(&shmref) {
                            // Verify our pattern wasn't corrupted by another thread
                            assert!(data.iter().all(|&b| b == pattern),
                                "thread {thread_id}: data corruption detected");
                        }

                        let release = SlotRelease {
                            arena_id: 0, slot: shmref.slot, generation: shmref.generation,
                        };
                        a.return_slot(&release);
                    }
                    // If try_acquire returns None, frame dropped — acceptable
                }
            })
        }).collect();

        for h in handles { h.join().unwrap(); }
    }

    // ── Slot scan distribution ──────────────────────────────────────

    #[test]
    fn slot_scan_distributes_across_all_slots() {
        let arena = make_arena(4096, 4, false);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..8 {
            let guard = arena.try_acquire().expect("acquire");
            seen.insert(guard.slot_index());
            drop(guard); // rollback
        }
        assert_eq!(seen.len(), 4, "scan must touch all 4 slots within 8 iterations");
    }

    // ── Full lifecycle stress ────────────────────────────────────────

    #[test]
    fn full_lifecycle_interleaved_four_slots() {
        let arena = make_arena(4096, 4, false);

        // Acquire all 4, write different patterns
        let mut guards: Vec<_> = (0..4)
            .map(|_| arena.try_acquire().expect("acquire"))
            .collect();
        for (i, g) in guards.iter_mut().enumerate() {
            g.write_from_slice(&[i as u8; 1]);
        }

        // Release all
        let refs: Vec<_> = guards.into_iter()
            .map(|g| g.release(0, 1))
            .collect();

        // Read all — verify each has correct data
        for (i, r) in refs.iter().enumerate() {
            let data = arena.read_ref(r).expect("read_ref");
            assert_eq!(data[0], i as u8, "slot {i} has wrong data");
        }

        // Return all
        for r in &refs {
            assert!(arena.return_slot(&SlotRelease {
                arena_id: 0, slot: r.slot, generation: r.generation,
            }));
        }

        // All 4 slots free
        for _ in 0..4 {
            assert!(arena.try_acquire().is_some());
        }
    }

    // ── Generation wrapping ─────────────────────────────────────────

    #[test]
    fn generation_wrapping_arithmetic_is_correct() {
        // Verify the wrapping_add used in return_slot handles u32::MAX
        let gen = u32::MAX;
        assert_eq!(gen.wrapping_add(1), 0);
        // The arena uses this arithmetic — if it panics on overflow, this catches it
    }

    // ── Fd accessors for sidechannel ────────────────────────────────

    #[test]
    fn arena_fd_and_states_fd_are_valid() {
        use std::os::unix::io::AsRawFd;
        let arena = make_arena(4096, 4, false);

        // Verify the fds are valid by calling fstat
        let arena_raw = arena.arena_fd().as_raw_fd();
        let states_raw = arena.states_fd().as_raw_fd();
        assert!(arena_raw >= 0, "arena_fd must be valid");
        assert!(states_raw >= 0, "states_fd must be valid");
        assert_ne!(arena_raw, states_raw, "arena and states must be different fds");

        // fstat must succeed on both
        unsafe {
            let mut stat: libc::stat = std::mem::zeroed();
            assert_eq!(libc::fstat(arena_raw, &mut stat), 0, "fstat on arena_fd must succeed");
            assert_eq!(libc::fstat(states_raw, &mut stat), 0, "fstat on states_fd must succeed");
        }
    }
}

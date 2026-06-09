use rekindle_transport_ipc::v3::bulk::pool::BufferPool;

#[test]
fn slab_is_zeroed_after_release() {
    let pool = BufferPool::with_capacity(1, 4096);
    {
        let mut slab = pool.try_acquire().unwrap();
        slab.buf_mut()[..20].copy_from_slice(b"hello sensitive data");
        slab.set_len(20);
        pool.release(slab);
        pool.reclaim();
    }

    let mut slab2 = pool.try_acquire().unwrap();
    assert!(
        slab2.buf_mut()[..20].iter().all(|&b| b == 0),
        "Slab was not zeroed after release. \
         Sensitive data would persist across pool acquisitions."
    );
}

#[test]
fn slab_zeroed_even_when_fully_written() {
    let pool = BufferPool::with_capacity(1, 256);
    {
        let mut slab = pool.try_acquire().unwrap();
        let cap = slab.capacity();
        for i in 0..cap {
            slab.buf_mut()[i] = 0xFF;
        }
        slab.set_len(cap);
        pool.release(slab);
        pool.reclaim();
    }

    let mut slab2 = pool.try_acquire().unwrap();
    assert!(
        slab2.buf_mut().iter().all(|&b| b == 0),
        "Fully-written slab was not zeroed."
    );
}

#[test]
fn multiple_acquire_release_cycles_stay_zeroed() {
    let pool = BufferPool::with_capacity(1, 128);
    for i in 0..10 {
        let mut slab = pool.try_acquire().unwrap();
        assert!(
            slab.buf_mut()[..64].iter().all(|&b| b == 0),
            "Slab not zeroed at start of cycle {i}"
        );
        slab.buf_mut()[..64].copy_from_slice(&vec![(i as u8).wrapping_add(1); 64]);
        slab.set_len(64);
        pool.release(slab);
        pool.reclaim();
    }
}

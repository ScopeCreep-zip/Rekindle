//! io_uring multishot recv with provided buffer rings for bulk receive.
//!
//! Uses `IORING_RECV_MULTISHOT` + `IOSQE_BUFFER_SELECT` for zero per-recv
//! SQE submission. The kernel picks buffers from the provided ring.
//!
//! # Buffer ring management
//!
//! Uses `io_uring::types::BufRingEntry` for type-safe access to the ring's
//! entry struct layout. This eliminates the raw pointer arithmetic that
//! would otherwise depend on kernel struct layout assumptions.
//!
//! # CQE handling rules
//!
//! - `IORING_CQE_F_MORE` set: multishot still active, more CQEs coming
//! - `IORING_CQE_F_MORE` absent: multishot terminated — rearm immediately
//! - `cqe.res == -ENOBUFS`: buffer ring exhausted — replenish and rearm
//! - `cqe.res == 0`: EOF (peer closed connection)
//! - `cqe.res < 0`: error
//! - Buffer ID: `cqe.flags >> IORING_CQE_BUFFER_SHIFT`

/// Configuration for the io_uring receive ring.
#[cfg(feature = "bulk-uring")]
#[derive(Clone, Copy)]
pub struct UringRecvConfig {
    /// Number of buffers in the provided buffer ring. Must be power of 2.
    pub num_buffers: u16,
    /// Size of each buffer in bytes.
    pub buffer_size: u32,
    /// Buffer group ID for `IOSQE_BUFFER_SELECT`.
    pub bgid: u16,
    /// SQ ring depth.
    pub sq_depth: u32,
}

#[cfg(feature = "bulk-uring")]
impl Default for UringRecvConfig {
    fn default() -> Self {
        Self {
            num_buffers: 512,
            buffer_size: 65600,
            bgid: 1,
            sq_depth: 64,
        }
    }
}

/// User data tag for the multishot recv SQE.
#[cfg(feature = "bulk-uring")]
const RECV_MULTI_TAG: u64 = 0xBEEF_0001;

/// RAII guard for page-aligned buffer memory.
#[cfg(feature = "bulk-uring")]
struct BufMemGuard {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

#[cfg(feature = "bulk-uring")]
impl Drop for BufMemGuard {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: ptr was allocated with this layout via alloc_zeroed.
            unsafe { std::alloc::dealloc(self.ptr, self.layout); }
        }
    }
}

/// Initialize buffer ring entries using the `io-uring` crate's `BufRingEntry` API.
///
/// This uses the crate's type-safe accessors (`set_addr`, `set_len`, `set_bid`)
/// instead of raw pointer arithmetic, eliminating assumptions about the kernel
/// struct layout.
///
/// # Safety
///
/// `ring_base` must point to page-aligned zeroed memory of at least
/// `num_buffers * size_of::<BufRingEntry>()` bytes.
/// `buf_base` must point to `num_buffers * buf_size` bytes.
#[cfg(feature = "bulk-uring")]
unsafe fn init_buf_ring(
    ring_base: *mut u8,
    buf_base: *const u8,
    num_buffers: u16,
    buf_size: u32,
) {
    use io_uring::types::BufRingEntry;

    let mask = num_buffers.wrapping_sub(1);
    // ring_base is page-aligned (4096 bytes) which exceeds BufRingEntry alignment (8).
    #[allow(clippy::cast_ptr_alignment)]
    let entries = ring_base.cast::<BufRingEntry>();

    for i in 0..num_buffers {
        let slot = (i & mask) as usize;
        // SAFETY: entries points to ring_base which has num_buffers entries.
        // slot < num_buffers because of the mask. The pointer is valid.
        let entry = unsafe { &mut *entries.add(slot) };
        // SAFETY: buf_base is valid for num_buffers * buf_size bytes.
        let addr = unsafe { buf_base.add(i as usize * buf_size as usize) } as u64;
        entry.set_addr(addr);
        entry.set_len(buf_size);
        entry.set_bid(i);
    }

    // Advance tail to make all entries visible to the kernel.
    // SAFETY: entries is valid, BufRingEntry::tail returns the tail
    // pointer from the ring header. write_volatile ensures the store
    // is not reordered past the fence.
    let tail_ptr = unsafe { BufRingEntry::tail(entries.cast_const()) };
    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
    // SAFETY: tail_ptr points to the ring tail field. write_volatile
    // ensures the store is visible to the kernel after the release fence.
    unsafe { tail_ptr.cast_mut().write_volatile(num_buffers) };
}

/// Return a buffer to the ring using the `BufRingEntry` API.
///
/// # Safety
///
/// `ring_base` must be the same pointer passed to `init_buf_ring`.
/// `bid` must be a valid buffer ID (< num_buffers).
/// `buf_base` must be the same pointer passed to `init_buf_ring`.
#[cfg(feature = "bulk-uring")]
unsafe fn return_buf_to_ring(
    ring_base: *mut u8,
    buf_base: *const u8,
    num_buffers: u16,
    buf_size: u32,
    bid: u16,
) {
    use io_uring::types::BufRingEntry;

    #[allow(clippy::cast_ptr_alignment)] // ring_base is page-aligned (4096 > 8)
    let entries = ring_base.cast::<BufRingEntry>();
    // SAFETY: entries is the ring base, BufRingEntry::tail returns the
    // tail pointer from the ring header union.
    let tail_ptr = unsafe { BufRingEntry::tail(entries.cast_const()) }.cast_mut();
    // SAFETY: tail_ptr is valid — derived from the ring base allocation.
    let tail = unsafe { tail_ptr.read_volatile() };
    let mask = num_buffers.wrapping_sub(1);
    let slot = (tail & mask) as usize;

    // SAFETY: slot < num_buffers (masked), entries allocation is valid.
    let entry = unsafe { &mut *entries.add(slot) };
    // SAFETY: bid < num_buffers (caller precondition), buf_base valid.
    let addr = unsafe { buf_base.add(bid as usize * buf_size as usize) } as u64;
    entry.set_addr(addr);
    entry.set_len(buf_size);
    entry.set_bid(bid);

    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
    // SAFETY: tail_ptr is valid, write_volatile ensures visibility.
    unsafe { tail_ptr.write_volatile(tail.wrapping_add(1)) };
}

/// Spawn the io_uring bulk receiver on a dedicated OS thread.
#[cfg(feature = "bulk-uring")]
pub fn spawn_uring_receiver(
    sock_fd: std::os::unix::io::RawFd,
    frame_tx: crossbeam::channel::Sender<Vec<u8>>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
    config: UringRecvConfig,
) -> std::thread::JoinHandle<std::io::Result<()>> {
    std::thread::Builder::new()
        .name("rekindle-uring-recv".into())
        .spawn(move || {
            use io_uring::{IoUring, opcode, types, squeue, cqueue};

            let mut ring = {
                let mut builder = IoUring::<squeue::Entry, cqueue::Entry>::builder();
                builder
                    .setup_single_issuer()
                    .setup_defer_taskrun()
                    .setup_coop_taskrun()
                    .setup_submit_all()
                    .setup_clamp()
                    .setup_cqsize(config.sq_depth * 4);

                if let Ok(r) = builder.build(config.sq_depth) { r } else {
                    let mut fallback = IoUring::<squeue::Entry, cqueue::Entry>::builder();
                    fallback
                        .setup_coop_taskrun()
                        .setup_submit_all()
                        .setup_clamp()
                        .setup_cqsize(config.sq_depth * 4);
                    fallback.build(config.sq_depth)?
                }
            };

            let buf_count = config.num_buffers as usize;
            let buf_size = config.buffer_size as usize;

            // Allocate buffer memory (page-aligned for THP promotion).
            let buf_total = buf_count * buf_size;
            let buf_layout = std::alloc::Layout::from_size_align(buf_total, 4096)
                .expect("valid buffer layout");
            // SAFETY: layout is valid and size > 0.
            let buf_base = unsafe { std::alloc::alloc_zeroed(buf_layout) };
            if buf_base.is_null() {
                return Err(std::io::Error::from(std::io::ErrorKind::OutOfMemory));
            }
            let _buf_guard = BufMemGuard { ptr: buf_base, layout: buf_layout };

            // Allocate buffer ring (page-aligned).
            let entry_size = std::mem::size_of::<types::BufRingEntry>();
            let ring_size = buf_count * entry_size;
            let ring_layout = std::alloc::Layout::from_size_align(ring_size, 4096)
                .expect("valid ring layout");
            // SAFETY: layout is valid and size > 0.
            let ring_base = unsafe { std::alloc::alloc_zeroed(ring_layout) };
            if ring_base.is_null() {
                return Err(std::io::Error::from(std::io::ErrorKind::OutOfMemory));
            }
            let _ring_guard = BufMemGuard { ptr: ring_base, layout: ring_layout };

            let bgid = config.bgid;

            // SAFETY: ring_base is valid page-aligned memory.
            unsafe {
                ring.submitter().register_buf_ring_with_flags(
                    ring_base as u64,
                    config.num_buffers,
                    bgid,
                    0,
                )?;
            }

            // SAFETY: ring_base and buf_base are valid, zeroed, page-aligned.
            unsafe {
                init_buf_ring(ring_base, buf_base, config.num_buffers, config.buffer_size);
            }

            let recv_sqe = opcode::RecvMulti::new(types::Fd(sock_fd), bgid)
                .build()
                .user_data(RECV_MULTI_TAG);

            // SAFETY: SQ is empty at startup.
            unsafe {
                let mut sq = ring.submission();
                sq.push(&recv_sqe).expect("SQ not full at startup");
            }
            ring.submit()?;

            let mut shutdown = shutdown;
            let mut enobufs_backoff_us: u64 = 0;

            loop {
                if shutdown.try_recv().is_ok() {
                    break;
                }

                match ring.submit_and_wait(1) {
                    Ok(_) => {}
                    Err(ref e) if e.raw_os_error() == Some(libc::ETIME) => continue,
                    Err(e) => return Err(e),
                }

                let mut need_rearm = false;

                // SAFETY: single issuer — no concurrent CQ access.
                let cq = unsafe { ring.completion_shared() };
                for cqe in cq {
                    let res = cqe.result();
                    let flags = cqe.flags();

                    if res == -libc::ENOBUFS {
                        // Buffer ring exhausted — no buffers available.
                        // Do NOT rearm immediately: the ring is empty and
                        // rearming would produce another ENOBUFS instantly,
                        // creating an infinite CPU-spinning loop.
                        // Exponential backoff: yield → 1µs → 10µs → 100µs → 1ms cap.
                        if enobufs_backoff_us == 0 {
                            std::thread::yield_now();
                            enobufs_backoff_us = 1;
                        } else {
                            std::thread::sleep(std::time::Duration::from_micros(enobufs_backoff_us));
                            enobufs_backoff_us = (enobufs_backoff_us * 10).min(1_000);
                        }
                        need_rearm = true;
                        continue;
                    }

                    if res <= 0 {
                        if res == 0 {
                            tracing::info!("uring recv: EOF (peer closed)");
                        } else {
                            tracing::warn!(error = res, "uring recv: error");
                        }
                        if !cqueue::more(flags) {
                            need_rearm = true;
                        }
                        continue;
                    }

                    let Some(bid) = cqueue::buffer_select(flags) else {
                        tracing::error!("uring recv: CQE has no buffer ID flag set — dropping frame");
                        if !cqueue::more(flags) {
                            need_rearm = true;
                        }
                        continue;
                    };
                    #[allow(clippy::cast_sign_loss)] // res > 0 checked above
                    let data_len = res as usize;

                    // Defense-in-depth: validate kernel-provided values before
                    // any pointer arithmetic. A malformed CQE must never reach
                    // unsafe code paths. Trust nothing from outside the process.
                    if bid >= config.num_buffers {
                        tracing::error!(
                            bid, num_buffers = config.num_buffers,
                            "uring recv: kernel returned out-of-range buffer ID — dropping frame"
                        );
                        if !cqueue::more(flags) {
                            need_rearm = true;
                        }
                        continue;
                    }
                    if data_len > buf_size {
                        tracing::error!(
                            data_len, buf_size,
                            "uring recv: kernel reported recv size exceeding buffer — dropping frame"
                        );
                        // SAFETY: ring_base, buf_base valid from alloc. bid < num_buffers
                        // (checked above). Buffer memory is valid even if data_len was wrong.
                        unsafe {
                            return_buf_to_ring(
                                ring_base, buf_base,
                                config.num_buffers, config.buffer_size, bid,
                            );
                        }
                        if !cqueue::more(flags) {
                            need_rearm = true;
                        }
                        continue;
                    }

                    // SAFETY: bid < num_buffers (checked above), data_len <= buf_size
                    // (checked above), buf_base is valid page-aligned allocation.
                    let buf_ptr = unsafe { buf_base.add(bid as usize * buf_size) };
                    let mut frame = vec![0u8; data_len];
                    // SAFETY: buf_ptr is valid for data_len bytes (bid < num_buffers,
                    // data_len <= buf_size, both checked above). frame is freshly
                    // allocated with exactly data_len bytes. No overlap.
                    unsafe {
                        std::ptr::copy_nonoverlapping(buf_ptr, frame.as_mut_ptr(), data_len);
                    }

                    // Reset ENOBUFS backoff — buffers are flowing.
                    enobufs_backoff_us = 0;

                    if frame_tx.send(frame).is_err() {
                        tracing::debug!("uring recv: frame channel closed");
                        break;
                    }

                    // Return buffer to ring using type-safe API.
                    // SAFETY: ring_base, buf_base, bid are all valid.
                    unsafe {
                        return_buf_to_ring(
                            ring_base, buf_base,
                            config.num_buffers, config.buffer_size,
                            bid,
                        );
                    }

                    if !cqueue::more(flags) {
                        need_rearm = true;
                    }
                }

                if need_rearm {
                    let sqe = opcode::RecvMulti::new(types::Fd(sock_fd), bgid)
                        .build()
                        .user_data(RECV_MULTI_TAG);
                    // SAFETY: single issuer, no concurrent SQ access.
                    unsafe {
                        let mut sq = ring.submission();
                        if sq.push(&sqe).is_err() {
                            drop(sq);
                            ring.submit()?;
                            let mut sq = ring.submission();
                            sq.push(&sqe).expect("SQ full after submit");
                        }
                    }
                }
            }

            let _ = ring.submitter().unregister_buf_ring(bgid);
            Ok(())
        })
        .expect("failed to spawn uring receiver thread")
}

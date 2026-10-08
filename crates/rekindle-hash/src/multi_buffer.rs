//! Multi-buffer SHA-256 through ISA-L's scheduler-level kernels.
//!
//! ISA-L hashes several independent messages at once, one per lane of a
//! vector register file, through a scheduler (`*_mb_mgr_init_*`,
//! `*_mb_mgr_submit_*`, `*_mb_mgr_flush_*`). Each [`Kernel`] is one of
//! ISA-L's scheduler variants built for this target: on x86_64, SSE and AVX
//! (4 lanes), AVX2 (8), AVX-512 (16) and the SHA-NI variants (SSE+SHA-NI
//! on 4 lanes; AVX-512 lanes whose flush finishes stragglers with SHA-NI);
//! on AArch64 Linux, the Armv8 crypto-extension kernel (3 lanes). A caller
//! may name any kernel the host supports ([`Kernel::is_supported`],
//! [`supported`]), take the one ISA-L's own dispatcher would select
//! ([`dispatched`]), or take the widest one a batch of a given size fills
//! ([`for_batch`]). On a target without the `sha256-mb` kernels the kernel
//! set is empty: [`supported`] returns none, and [`dispatched`] and
//! [`for_batch`] `None`.
//!
//! # Why the scheduler, not the context manager
//!
//! ISA-L's context manager (`isal_sha256_ctx_mgr_submit`) with
//! `ISAL_HASH_ENTIRE` enters a resubmit loop that hashes all blocks of one
//! message before returning, so its lanes are never occupied together. The
//! scheduler places each submitted job in a lane and runs the kernel only
//! once every lane holds work: true N-way parallelism across messages.
//! Bypassing the context manager means this module does what it would:
//! padding, the initial digest, and resubmitting a job until its message
//! is complete.
//!
//! # Zero-copy submission
//!
//! Messages are hashed in place: their whole blocks are submitted straight
//! from the caller's buffers, and only the final partial block and the
//! padding (at most two blocks) are written to a per-message buffer and
//! submitted as the message's continuation, the job's digest carrying the
//! running state between submissions. A message longer than the kernel's
//! per-submission limit ([`Kernel::max_submit_blocks`]) is submitted in
//! segments the same way.
//!
//! # Allocation
//!
//! The scheduler's jobs, the padded tails and each message's progress live
//! in a [`Workspace`] that is cleared, not freed, between calls, so it
//! allocates only when a batch is larger than every batch before it. A
//! caller that hashes repeatedly owns one ([`sha256_mb_with`]);
//! [`sha256_mb`] uses one per thread. Nothing in a workspace outlives the
//! call that filled it, so no cross-thread pool or completion gate is
//! involved: the steady state performs no allocation and no atomic
//! operation.
//!
//! # Safety and ABI
//!
//! `Job` mirrors `ISAL_SHA256_JOB` field for field, and `JobMgr` is an
//! opaque, exactly sized `ISAL_SHA256_MB_JOB_MGR`. Their sizes and the
//! job's field offsets are defined once in build.rs, checked there against
//! ISA-L's headers with C `_Static_assert`s, and checked here against the
//! Rust types with `const` assertions: a layout mismatch fails the build.
//! The kernels read the job fields at fixed offsets (`sha256_job.asm`,
//! `sha256_mb_x1_ce.S`) and keep the manager's lanes vector-aligned, so
//! both are 64-byte aligned. A kernel runs only after
//! [`Kernel::is_supported`] confirms the host's CPU and OS support its
//! instructions; build.rs guarantees the x86_64 C in front of the assembly
//! needs nothing beyond the target's baseline.

use std::fmt;

/// One of ISA-L's multi-buffer SHA-256 scheduler variants built for this
/// target. Uninhabited on a target with none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kernel {
    /// 4 lanes of SSE (`sha256_mb_x4_sse`).
    #[cfg(isal_x86_64)]
    Sse,
    /// 4 lanes of AVX (`sha256_mb_x4_avx`).
    #[cfg(isal_x86_64)]
    Avx,
    /// 8 lanes of AVX2 (`sha256_mb_x8_avx2`).
    #[cfg(isal_x86_64)]
    Avx2,
    /// 16 lanes of AVX-512 (`sha256_mb_x16_avx512`).
    #[cfg(isal_x86_64)]
    Avx512,
    /// SHA-NI, 2 messages per pass, on the 4-lane SSE scheduler
    /// (`sha256_ni_x1`, `sha256_ni_x2`).
    #[cfg(isal_x86_64)]
    SseNi,
    /// The AVX-512 scheduler whose flush hashes with SHA-NI once few lanes
    /// remain occupied.
    #[cfg(isal_x86_64)]
    Avx512Ni,
    /// 3 lanes of Armv8 SHA-2 crypto instructions (`sha256_mb_x3_ce`).
    #[cfg(isal_aarch64)]
    Ce,
}

impl Kernel {
    /// Every kernel built for this target, narrowest first.
    #[cfg(isal_x86_64)]
    pub const ALL: &'static [Self] = &[
        Self::Sse,
        Self::Avx,
        Self::Avx2,
        Self::Avx512,
        Self::SseNi,
        Self::Avx512Ni,
    ];
    /// Every kernel built for this target, narrowest first.
    #[cfg(isal_aarch64)]
    pub const ALL: &'static [Self] = &[Self::Ce];
    /// Every kernel built for this target: none.
    #[cfg(not(any(isal_x86_64, isal_aarch64)))]
    pub const ALL: &'static [Self] = &[];

    /// A stable lowercase name: `sse`, `avx`, `avx2`, `avx512`, `sse_ni`,
    /// `avx512_ni`, `ce`.
    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(isal_x86_64)]
            Self::Sse => "sse",
            #[cfg(isal_x86_64)]
            Self::Avx => "avx",
            #[cfg(isal_x86_64)]
            Self::Avx2 => "avx2",
            #[cfg(isal_x86_64)]
            Self::Avx512 => "avx512",
            #[cfg(isal_x86_64)]
            Self::SseNi => "sse_ni",
            #[cfg(isal_x86_64)]
            Self::Avx512Ni => "avx512_ni",
            #[cfg(isal_aarch64)]
            Self::Ce => "ce",
        }
    }

    /// Messages the kernel hashes per pass.
    pub const fn lanes(self) -> usize {
        match self {
            #[cfg(isal_x86_64)]
            Self::Sse | Self::Avx | Self::SseNi => 4,
            #[cfg(isal_x86_64)]
            Self::Avx2 => 8,
            #[cfg(isal_x86_64)]
            Self::Avx512 | Self::Avx512Ni => 16,
            #[cfg(isal_aarch64)]
            Self::Ce => 3,
        }
    }

    /// Blocks one submission may carry. The x86_64 schedulers keep a job's
    /// remaining length in the upper 28 bits of a 32-bit lane word, the
    /// lane index in the low nibble (`sha256_mb_mgr_submit_*.asm`:
    /// `shl len, 4; or len, lane`). The AArch64 scheduler holds the same
    /// word in a signed `int` and advances the buffer by `len << 2`
    /// (`sha256_mb_mgr_ce.c`), so blocks shifted left by 6 must stay below
    /// 2^31.
    pub const fn max_submit_blocks(self) -> usize {
        match self {
            #[cfg(isal_x86_64)]
            Self::Sse
            | Self::Avx
            | Self::Avx2
            | Self::Avx512
            | Self::SseNi
            | Self::Avx512Ni => (1 << 28) - 1,
            #[cfg(isal_aarch64)]
            Self::Ce => (1 << 25) - 1,
        }
    }

    /// Whether this host can run the kernel. On x86_64: the conditions
    /// under which ISA-L's multibinary dispatcher
    /// (`mbin_dispatch_base_to_avx512_shani`, `include/multibinary.asm`)
    /// reaches it, with OS support for the register state included
    /// (`is_x86_feature_detected!` checks XCR0). On AArch64: `HWCAP_SHA2`,
    /// as ISA-L's dispatcher checks (`sha256_mb_aarch64_dispatcher.c`).
    pub fn is_supported(self) -> bool {
        match self {
            #[cfg(isal_x86_64)]
            Self::Sse => x86::sse(),
            #[cfg(isal_x86_64)]
            Self::Avx => x86::avx(),
            #[cfg(isal_x86_64)]
            Self::Avx2 => x86::avx2(),
            #[cfg(isal_x86_64)]
            Self::Avx512 => x86::avx512(),
            #[cfg(isal_x86_64)]
            Self::SseNi => x86::sse() && x86::sha(),
            #[cfg(isal_x86_64)]
            Self::Avx512Ni => x86::avx512() && x86::sha(),
            #[cfg(isal_aarch64)]
            Self::Ce => std::arch::is_aarch64_feature_detected!("sha2"),
        }
    }
}

impl fmt::Display for Kernel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The kernels this host supports, narrowest first.
pub fn supported() -> Vec<Kernel> {
    Kernel::ALL
        .iter()
        .copied()
        .filter(|kernel| kernel.is_supported())
        .collect()
}

/// The kernel ISA-L's dispatcher selects on this host, or none where ISA-L
/// falls back to portable C.
///
/// On x86_64, none without SSE4.2. Without usable AVX: SSE+SHA-NI with
/// SHA-NI, else SSE. With AVX: AVX, or AVX2 with AVX2, or AVX-512 with
/// AVX-512 F/VL/BW/CD/DQ, and then AVX-512+SHA-NI with SHA-NI. ISA-L also
/// takes its AVX-512+SHA-NI path on a host that enables AVX-512 register
/// state and has SHA-NI but lacks part of that group; that path executes
/// AVX-512 instructions, so here it requires the group.
///
/// On AArch64, the crypto-extension kernel with `HWCAP_SHA2`.
pub fn dispatched() -> Option<Kernel> {
    #[cfg(isal_x86_64)]
    {
        if !Kernel::Sse.is_supported() {
            return None;
        }
        if !Kernel::Avx.is_supported() {
            return Some(if x86::sha() {
                Kernel::SseNi
            } else {
                Kernel::Sse
            });
        }
        if !Kernel::Avx2.is_supported() {
            return Some(Kernel::Avx);
        }
        if !Kernel::Avx512.is_supported() {
            return Some(Kernel::Avx2);
        }
        Some(if x86::sha() {
            Kernel::Avx512Ni
        } else {
            Kernel::Avx512
        })
    }
    #[cfg(isal_aarch64)]
    {
        Kernel::Ce.is_supported().then_some(Kernel::Ce)
    }
    #[cfg(not(any(isal_x86_64, isal_aarch64)))]
    {
        None
    }
}

/// Kernels in ISA-L's order of preference among those of equal width:
/// AVX-512+SHA-NI over AVX-512, AVX over SSE+SHA-NI over SSE (with AVX
/// usable ISA-L never selects a SHA-NI kernel narrower than AVX-512).
#[cfg(isal_x86_64)]
const PREFERENCE: &[Kernel] = &[
    Kernel::Avx512Ni,
    Kernel::Avx512,
    Kernel::Avx2,
    Kernel::Avx,
    Kernel::SseNi,
    Kernel::Sse,
];
#[cfg(isal_aarch64)]
const PREFERENCE: &[Kernel] = &[Kernel::Ce];
#[cfg(not(any(isal_x86_64, isal_aarch64)))]
const PREFERENCE: &[Kernel] = &[];

/// The kernel for a batch of `messages`: the first supported kernel in
/// ISA-L's preference order whose lanes the batch fills, or none when the
/// batch fills no supported kernel's lanes. For a batch at least as wide
/// as the widest supported kernel this is [`dispatched`]; a narrower batch
/// gets the widest kernel it fills (8 messages on an AVX-512 host: AVX2).
pub fn for_batch(messages: usize) -> Option<Kernel> {
    PREFERENCE
        .iter()
        .copied()
        .find(|kernel| kernel.lanes() <= messages && kernel.is_supported())
}

/// Why a multi-buffer hash was not computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The host cannot run the kernel.
    Unsupported(Kernel),
    /// One digest slot per message is required.
    LengthMismatch { messages: usize, digests: usize },
    /// The message exceeds SHA-256's 2^64 - 1 bit length.
    MessageTooLong { index: usize },
    /// The scheduler returned fewer finished messages than were submitted.
    Incomplete { finished: usize, messages: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(kernel) => {
                write!(f, "this host cannot run the {kernel} SHA-256 kernel")
            }
            Self::LengthMismatch { messages, digests } => {
                write!(f, "{messages} messages but {digests} digest slots")
            }
            Self::MessageTooLong { index } => {
                write!(f, "message {index} exceeds the SHA-256 length limit")
            }
            Self::Incomplete { finished, messages } => write!(
                f,
                "the scheduler finished {finished} of {messages} messages"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Reusable scratch for multi-buffer hashing: the scheduler's jobs, the
/// padded message tails and each message's progress. Cleared, not freed,
/// between calls; it grows only when a batch exceeds every earlier batch.
#[derive(Default)]
pub struct Workspace {
    #[cfg(any(isal_x86_64, isal_aarch64))]
    scratch: scheduler::Scratch,
}

impl Workspace {
    /// An empty workspace; it allocates on first use.
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for Workspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workspace").finish_non_exhaustive()
    }
}

/// SHA-256 of each message in `messages` into the matching slot of
/// `digests`, through `kernel`, using this thread's [`Workspace`].
#[cfg_attr(not(any(isal_x86_64, isal_aarch64)), allow(unused_variables))]
pub fn sha256_mb(
    kernel: Kernel,
    messages: &[&[u8]],
    digests: &mut [[u8; 32]],
) -> Result<(), Error> {
    #[cfg(any(isal_x86_64, isal_aarch64))]
    {
        thread_local! {
            static WORKSPACE: std::cell::RefCell<Workspace> =
                std::cell::RefCell::new(Workspace::new());
        }
        WORKSPACE.with(|workspace| {
            sha256_mb_with(&mut workspace.borrow_mut(), kernel, messages, digests)
        })
    }
    #[cfg(not(any(isal_x86_64, isal_aarch64)))]
    {
        match kernel {}
    }
}

/// [`sha256_mb`] with a caller-owned [`Workspace`].
#[cfg_attr(not(any(isal_x86_64, isal_aarch64)), allow(unused_variables))]
pub fn sha256_mb_with(
    workspace: &mut Workspace,
    kernel: Kernel,
    messages: &[&[u8]],
    digests: &mut [[u8; 32]],
) -> Result<(), Error> {
    #[cfg(any(isal_x86_64, isal_aarch64))]
    {
        scheduler::hash(
            &mut workspace.scratch,
            kernel,
            messages,
            digests,
            kernel.max_submit_blocks(),
        )
    }
    #[cfg(not(any(isal_x86_64, isal_aarch64)))]
    {
        match kernel {}
    }
}

#[cfg(isal_x86_64)]
mod x86 {
    pub fn sse() -> bool {
        std::is_x86_feature_detected!("sse4.2")
    }

    pub fn avx() -> bool {
        sse() && std::is_x86_feature_detected!("avx")
    }

    pub fn avx2() -> bool {
        avx() && std::is_x86_feature_detected!("avx2")
    }

    /// AVX-512 F, VL, BW, CD and DQ: ISA-L's `FLAGS_CPUID7_EBX_AVX512_G1`.
    pub fn avx512() -> bool {
        avx2()
            && std::is_x86_feature_detected!("avx512f")
            && std::is_x86_feature_detected!("avx512vl")
            && std::is_x86_feature_detected!("avx512bw")
            && std::is_x86_feature_detected!("avx512cd")
            && std::is_x86_feature_detected!("avx512dq")
    }

    pub fn sha() -> bool {
        std::is_x86_feature_detected!("sha")
    }
}

#[cfg(any(isal_x86_64, isal_aarch64))]
mod scheduler {
    use super::{Error, Kernel};

    mod layout {
        include!(concat!(env!("OUT_DIR"), "/isal_layout.rs"));
    }

    const BLOCK: usize = 64;

    /// SHA-256 initial hash value (FIPS 180-4 §5.3.3).
    const INITIAL_DIGEST: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    /// `ISAL_SHA256_MB_JOB_MGR`: opaque to Rust, sized and checked by
    /// build.rs.
    #[repr(C, align(64))]
    pub struct JobMgr([u8; layout::MGR_SIZE]);

    /// `ISAL_SHA256_JOB`.
    #[repr(C, align(64))]
    pub struct Job {
        buffer: *const u8,
        /// Blocks to hash from `buffer`.
        len: u64,
        _pad0: [u8; 48],
        /// Running state in, digest out, as host-order words.
        digest: [u32; 8],
        status: u32,
        _pad1: u32,
        /// The message index this job hashes.
        user_data: u64,
        _pad2: [u8; 16],
    }

    const _: () = {
        assert!(std::mem::size_of::<Job>() == layout::JOB_SIZE);
        assert!(std::mem::offset_of!(Job, buffer) == layout::JOB_BUFFER_OFFSET);
        assert!(std::mem::offset_of!(Job, len) == layout::JOB_LEN_OFFSET);
        assert!(std::mem::offset_of!(Job, digest) == layout::JOB_DIGEST_OFFSET);
        assert!(std::mem::offset_of!(Job, status) == layout::JOB_STATUS_OFFSET);
        assert!(std::mem::offset_of!(Job, user_data) == layout::JOB_USER_DATA_OFFSET);
    };

    impl Job {
        const fn new(index: u64) -> Self {
            Self {
                buffer: std::ptr::null(),
                len: 0,
                _pad0: [0; 48],
                digest: INITIAL_DIGEST,
                status: 0,
                _pad1: 0,
                user_data: index,
                _pad2: [0; 16],
            }
        }
    }

    type Init = unsafe extern "C" fn(*mut JobMgr);
    type Submit = unsafe extern "C" fn(*mut JobMgr, *mut Job) -> *mut Job;
    type Flush = unsafe extern "C" fn(*mut JobMgr) -> *mut Job;

    #[cfg(isal_x86_64)]
    extern "C" {
        fn _sha256_mb_mgr_init_sse(state: *mut JobMgr);
        fn _sha256_mb_mgr_submit_sse(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn _sha256_mb_mgr_flush_sse(state: *mut JobMgr) -> *mut Job;
        fn _sha256_mb_mgr_submit_avx(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn _sha256_mb_mgr_flush_avx(state: *mut JobMgr) -> *mut Job;
        fn _sha256_mb_mgr_init_avx2(state: *mut JobMgr);
        fn _sha256_mb_mgr_submit_avx2(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn _sha256_mb_mgr_flush_avx2(state: *mut JobMgr) -> *mut Job;
        fn _sha256_mb_mgr_init_avx512(state: *mut JobMgr);
        fn _sha256_mb_mgr_submit_avx512(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn _sha256_mb_mgr_flush_avx512(state: *mut JobMgr) -> *mut Job;
        fn _sha256_mb_mgr_submit_sse_ni(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn _sha256_mb_mgr_flush_sse_ni(state: *mut JobMgr) -> *mut Job;
        fn _sha256_mb_mgr_flush_avx512_ni(state: *mut JobMgr) -> *mut Job;
    }

    #[cfg(isal_aarch64)]
    extern "C" {
        fn sha256_mb_mgr_init_ce(state: *mut JobMgr);
        fn sha256_mb_mgr_submit_ce(state: *mut JobMgr, job: *mut Job) -> *mut Job;
        fn sha256_mb_mgr_flush_ce(state: *mut JobMgr) -> *mut Job;
    }

    const fn entry(kernel: Kernel) -> (Init, Submit, Flush) {
        match kernel {
            #[cfg(isal_x86_64)]
            Kernel::Sse => (
                _sha256_mb_mgr_init_sse,
                _sha256_mb_mgr_submit_sse,
                _sha256_mb_mgr_flush_sse,
            ),
            // ISA-L defines `_sha256_mb_mgr_init_avx` as the SSE init.
            #[cfg(isal_x86_64)]
            Kernel::Avx => (
                _sha256_mb_mgr_init_sse,
                _sha256_mb_mgr_submit_avx,
                _sha256_mb_mgr_flush_avx,
            ),
            #[cfg(isal_x86_64)]
            Kernel::Avx2 => (
                _sha256_mb_mgr_init_avx2,
                _sha256_mb_mgr_submit_avx2,
                _sha256_mb_mgr_flush_avx2,
            ),
            #[cfg(isal_x86_64)]
            Kernel::Avx512 => (
                _sha256_mb_mgr_init_avx512,
                _sha256_mb_mgr_submit_avx512,
                _sha256_mb_mgr_flush_avx512,
            ),
            // `sha256_ctx_sse_ni.c`: SSE manager, SHA-NI submit and flush.
            #[cfg(isal_x86_64)]
            Kernel::SseNi => (
                _sha256_mb_mgr_init_sse,
                _sha256_mb_mgr_submit_sse_ni,
                _sha256_mb_mgr_flush_sse_ni,
            ),
            // `sha256_ctx_avx512_ni.c`: AVX-512 manager and submit, SHA-NI
            // flush.
            #[cfg(isal_x86_64)]
            Kernel::Avx512Ni => (
                _sha256_mb_mgr_init_avx512,
                _sha256_mb_mgr_submit_avx512,
                _sha256_mb_mgr_flush_avx512_ni,
            ),
            #[cfg(isal_aarch64)]
            Kernel::Ce => (
                sha256_mb_mgr_init_ce,
                sha256_mb_mgr_submit_ce,
                sha256_mb_mgr_flush_ce,
            ),
        }
    }

    /// A message's progress through the scheduler: whole blocks from the
    /// caller's buffer, then its padded tail.
    struct Cursor {
        /// Bytes of whole blocks in the message.
        body: usize,
        /// Bytes of the body already submitted.
        submitted: usize,
        tail_blocks: u64,
        tail_submitted: bool,
    }

    impl Cursor {
        /// Split `data` into its whole blocks and the padded tail written to
        /// `tail`: the remaining bytes, `0x80`, zeros, and the bit length as
        /// a big-endian 64-bit integer, in one block or, when the remainder
        /// leaves fewer than 9 bytes, two.
        fn new(data: &[u8], tail: &mut [u8; 2 * BLOCK]) -> Option<Self> {
            let bits = u64::try_from(data.len()).ok()?.checked_mul(8)?;
            let body = data.len() - data.len() % BLOCK;
            let remainder = &data[body..];
            let tail_len = if remainder.len() < BLOCK - 8 {
                BLOCK
            } else {
                2 * BLOCK
            };
            tail[..remainder.len()].copy_from_slice(remainder);
            tail[remainder.len()] = 0x80;
            tail[remainder.len() + 1..tail_len - 8].fill(0);
            tail[tail_len - 8..tail_len].copy_from_slice(&bits.to_be_bytes());
            Some(Self {
                body,
                submitted: 0,
                tail_blocks: if tail_len == BLOCK { 1 } else { 2 },
                tail_submitted: false,
            })
        }

        /// The next segment of `data` to submit: up to `limit` whole blocks
        /// of the body, then the tail (`tail`), then none once the message
        /// is finished.
        fn next(&mut self, data: &[u8], tail: *const u8, limit: usize) -> Option<(*const u8, u64)> {
            let remaining = (self.body - self.submitted) / BLOCK;
            if remaining > 0 {
                let blocks = remaining.min(limit);
                let start = data[self.submitted..].as_ptr();
                self.submitted += blocks * BLOCK;
                return Some((start, blocks as u64));
            }
            if self.tail_submitted {
                return None;
            }
            self.tail_submitted = true;
            Some((tail, self.tail_blocks))
        }
    }

    /// The reusable buffers behind a [`super::Workspace`].
    #[derive(Default)]
    pub struct Scratch {
        jobs: Vec<Job>,
        tails: Vec<[u8; 2 * BLOCK]>,
        cursors: Vec<Cursor>,
    }

    /// [`super::sha256_mb_with`] with submissions of at most `limit`
    /// blocks.
    pub fn hash(
        scratch: &mut Scratch,
        kernel: Kernel,
        messages: &[&[u8]],
        digests: &mut [[u8; 32]],
        limit: usize,
    ) -> Result<(), Error> {
        if messages.len() != digests.len() {
            return Err(Error::LengthMismatch {
                messages: messages.len(),
                digests: digests.len(),
            });
        }
        if !kernel.is_supported() {
            return Err(Error::Unsupported(kernel));
        }
        if messages.is_empty() {
            return Ok(());
        }
        let count = messages.len();
        let Scratch {
            jobs,
            tails,
            cursors,
        } = scratch;
        // Every buffer the scheduler is handed is in place before the first
        // submission, and none moves until the last job completes.
        tails.resize(count, [0; 2 * BLOCK]);
        cursors.clear();
        for (index, (data, tail)) in messages.iter().zip(tails.iter_mut()).enumerate() {
            cursors.push(Cursor::new(data, tail).ok_or(Error::MessageTooLong { index })?);
        }
        jobs.clear();
        jobs.extend((0..count as u64).map(Job::new));

        let mut manager = JobMgr([0; layout::MGR_SIZE]);
        let (init, submit, flush) = entry(kernel);
        let tail_base = tails.as_ptr().cast::<u8>();
        let job_base = jobs.as_mut_ptr();
        let manager = &raw mut manager;

        let mut finished = 0;
        // SAFETY: the kernel is supported by this host (checked above), so
        // its init, submit and flush execute only instructions the host
        // has. The manager is initialized by `init` before any submit.
        // Every job pointer is `job_base.add(i)` for `i < count` within
        // `jobs`, which is not resized, moved or dropped until after the
        // last flush; each job is written only while the scheduler does not
        // hold it (before its first submission, or after the scheduler
        // returned it). Every buffer a job points at is a whole-block
        // segment of a caller's message, borrowed for this call, or that
        // message's tail in `tails`, which likewise is not resized, moved
        // or dropped, and covers `len` blocks, at most the kernel's
        // submission limit.
        // SAFETY: see the block comment above `init(manager)` below.
        unsafe {
            init(manager);

            /// Resubmit or retire every job the scheduler returns until it
            /// wants more input (returns null).
            ///
            /// # Safety
            ///
            /// `done` is null or points into `jobs`. `cursors`, `messages`
            /// and `tails` (via `tail_base`) are indexed by the job's
            /// `user_data`, which is `0..count`.
            unsafe fn drain(
                done: *mut Job,
                submit: Submit,
                manager: *mut JobMgr,
                cursors: &mut [Cursor],
                messages: &[&[u8]],
                tail_base: *const u8,
                limit: usize,
                finished: &mut usize,
                count: usize,
            ) -> Result<(), Error> {
                let mut done = done;
                while !done.is_null() {
                    // SAFETY: `done` points into `jobs`, whose `user_data`
                    // is the message index set at construction.
                    let index = usize::try_from(unsafe { (*done).user_data }).map_err(|_| {
                        Error::Incomplete {
                            finished: *finished,
                            messages: count,
                        }
                    })?;
                    // SAFETY: `index < count`, `tail_base` is the start of
                    // `tails` which has `count` entries of `2 * BLOCK` each.
                    let tail = unsafe { tail_base.add(index * 2 * BLOCK) };
                    if let Some((buffer, blocks)) = cursors[index].next(messages[index], tail, limit) {
                        // SAFETY: `done` points into `jobs`.
                        unsafe {
                            (*done).buffer = buffer;
                            (*done).len = blocks;
                            done = submit(manager, done);
                        }
                    } else {
                        *finished += 1;
                        done = std::ptr::null_mut();
                    }
                }
                Ok(())
            }

            for index in 0..count {
                let job = job_base.add(index);
                let tail = tail_base.add(index * 2 * BLOCK);
                let (buffer, blocks) = cursors[index]
                    .next(messages[index], tail, limit)
                    .ok_or(Error::Incomplete {
                        finished: 0,
                        messages: count,
                    })?;
                (*job).buffer = buffer;
                (*job).len = blocks;
                let returned = submit(manager, job);
                drain(
                    returned, submit, manager, cursors, messages, tail_base,
                    limit, &mut finished, count,
                )?;
            }
            loop {
                let done = flush(manager);
                if done.is_null() {
                    break;
                }
                drain(
                    done, submit, manager, cursors, messages, tail_base,
                    limit, &mut finished, count,
                )?;
            }
        }
        if finished != count {
            return Err(Error::Incomplete {
                finished,
                messages: count,
            });
        }
        for (job, digest) in jobs.iter().zip(digests.iter_mut()) {
            for (word, out) in job.digest.iter().zip(digest.chunks_exact_mut(4)) {
                out.copy_from_slice(&word.to_be_bytes());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernels this host supports, as the emulated-CPU matrix expects
    /// them (`REKINDLE_HASH_EXPECT_KERNELS`, comma-separated names); the
    /// matrix sets it per CPU model so a model that silently loses or
    /// gains a kernel fails.
    #[test]
    fn supported_kernels_match_the_expected_set_when_one_is_given() {
        let Ok(expected) = std::env::var("REKINDLE_HASH_EXPECT_KERNELS") else {
            return;
        };
        let names: Vec<&str> = supported().iter().map(|kernel| kernel.name()).collect();
        let expected: Vec<&str> = expected.split(',').filter(|name| !name.is_empty()).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn the_dispatched_kernel_is_supported_and_the_widest_isa_l_selects() {
        let kernels = supported();
        match dispatched() {
            None => assert!(kernels.is_empty()),
            Some(kernel) => {
                assert!(kernels.contains(&kernel));
                #[cfg(isal_x86_64)]
                {
                    if Kernel::Avx512.is_supported() {
                        assert!(matches!(kernel, Kernel::Avx512 | Kernel::Avx512Ni));
                    } else if Kernel::Avx2.is_supported() {
                        assert_eq!(kernel, Kernel::Avx2);
                    }
                }
            }
        }
    }

    /// A batch gets the preferred kernel whose lanes it fills: none below
    /// the narrowest supported kernel's width, [`dispatched`] at or above
    /// the widest, and never a kernel wider than the batch.
    #[test]
    fn a_batch_gets_the_widest_supported_kernel_it_fills() {
        let kernels = supported();
        let narrowest = kernels.iter().map(|kernel| kernel.lanes()).min();
        let widest = kernels.iter().map(|kernel| kernel.lanes()).max();
        for messages in 0..=64 {
            let chosen = for_batch(messages);
            match chosen {
                None => assert!(narrowest.is_none_or(|lanes| messages < lanes)),
                Some(kernel) => {
                    assert!(kernel.is_supported());
                    let best = kernels
                        .iter()
                        .map(|candidate| candidate.lanes())
                        .filter(|&lanes| lanes <= messages)
                        .max();
                    assert_eq!(Some(kernel.lanes()), best, "{messages} messages: {kernel}");
                }
            }
            if widest.is_some_and(|lanes| messages >= lanes) {
                assert_eq!(chosen, dispatched(), "{messages} messages");
            }
        }
    }

    #[cfg(any(isal_x86_64, isal_aarch64))]
    mod kernels {
        use super::super::scheduler::{hash, Scratch};
        use super::super::*;
        use crate::single::sha256_oneshot;

        /// Lengths around every padding boundary: empty, one byte, the last
        /// length with a one-block tail (55), the first needing two (56),
        /// the block edges, multi-block, and a ragged large message.
        const LENGTHS: [usize; 14] = [
            0,
            1,
            55,
            56,
            63,
            64,
            65,
            119,
            120,
            127,
            128,
            1000,
            65_519,
            (1 << 20) + 7,
        ];

        fn data(length: usize, seed: usize) -> Vec<u8> {
            (0..length)
                .map(|i| u8::try_from((i * 31 + seed * 7) % 251).unwrap_or(0))
                .collect()
        }

        fn check(scratch: &mut Scratch, kernel: Kernel, messages: &[Vec<u8>], limit: usize) {
            let refs: Vec<&[u8]> = messages.iter().map(Vec::as_slice).collect();
            let mut digests = vec![[0u8; 32]; refs.len()];
            hash(scratch, kernel, &refs, &mut digests, limit)
                .unwrap_or_else(|error| panic!("{kernel}: {error}"));
            for (index, (message, digest)) in refs.iter().zip(&digests).enumerate() {
                assert_eq!(
                    *digest,
                    sha256_oneshot(message),
                    "{kernel}: message {index} of {} ({} bytes), limit {limit}",
                    refs.len(),
                    message.len()
                );
            }
        }

        /// Every supported kernel agrees with aws-lc at every padding
        /// boundary, for batches below, at and above its lane count, all
        /// lengths mixed in one batch, and with submissions split into 1-
        /// and 3-block segments. One scratch serves every batch, so reuse
        /// across shrinking and growing batches is exercised too.
        #[test]
        fn every_supported_kernel_matches_single_buffer_sha256() {
            // A host with no supported kernel (x86_64 without SSE4.2)
            // checks nothing here; the emulated-CPU matrix pins which
            // kernels each CPU model must support.
            let kernels = supported();
            let mut scratch = Scratch::default();
            for kernel in kernels {
                for &length in &LENGTHS {
                    for count in [
                        2 * kernel.lanes() + 3,
                        1,
                        3,
                        kernel.lanes(),
                        kernel.lanes() + 1,
                    ] {
                        let messages: Vec<Vec<u8>> =
                            (0..count).map(|seed| data(length, seed)).collect();
                        check(&mut scratch, kernel, &messages, kernel.max_submit_blocks());
                    }
                }
                let mixed: Vec<Vec<u8>> = LENGTHS
                    .iter()
                    .enumerate()
                    .map(|(seed, &length)| data(length, seed))
                    .collect();
                check(&mut scratch, kernel, &mixed, kernel.max_submit_blocks());
                check(&mut scratch, kernel, &mixed, 1);
                check(&mut scratch, kernel, &mixed, 3);
            }
        }

        #[test]
        fn unsupported_kernels_and_mismatched_slots_are_refused() {
            for &kernel in Kernel::ALL {
                let mut digests = [[0u8; 32]; 1];
                let result = sha256_mb(kernel, &[b"abc"], &mut digests);
                if kernel.is_supported() {
                    assert_eq!(result, Ok(()));
                    assert_eq!(digests[0], sha256_oneshot(b"abc"));
                } else {
                    assert_eq!(result, Err(Error::Unsupported(kernel)));
                }
            }
            if let Some(kernel) = dispatched() {
                let mut digests = [[0u8; 32]; 2];
                assert_eq!(
                    sha256_mb(kernel, &[b"abc"], &mut digests),
                    Err(Error::LengthMismatch {
                        messages: 1,
                        digests: 2
                    })
                );
            }
        }
    }
}

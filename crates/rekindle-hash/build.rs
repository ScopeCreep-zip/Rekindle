//! Builds ISA-L's multi-buffer SHA-256 scheduler kernels with the
//! `sha256-mb` feature: the x86_64 kernels (SSE, AVX, AVX2, AVX-512 and
//! SHA-NI) and, on AArch64 Linux, the Armv8 crypto-extension kernel. The
//! target is read from Cargo's `CARGO_CFG_TARGET_*` variables (a build
//! script's own `cfg` describes the build host), and the kernels built are
//! announced to the crate as `isal_x86_64` or `isal_aarch64`.
//!
//! The C is compiled for the target's baseline ISA, as ISA-L's own build
//! compiles it (`cmake/sha256_mb.cmake`, `CMakeLists.txt`): every
//! ISA-specific instruction lives in assembly kernels that callers select
//! at run time. An `-m` ISA flag on the C would license the compiler to
//! emit that ISA anywhere in every C file (with `-mavx512f`, GCC lowers the
//! `memset` in `_sha256_mb_mgr_init_avx2` to zmm stores, and the AVX2 path
//! faults with SIGILL on AVX2 hosts without AVX-512). On x86_64, every
//! instruction of the compiled C is decoded and checked against the
//! target's ISA, and one beyond it fails the build. The AArch64 C is the
//! scheduler alone (`sha256_mb_mgr_ce.c`), built with no ISA flags; the
//! decoder that audits x86_64 does not decode AArch64.
//!
//! The scheduler ABI the Rust side calls (`ISAL_SHA256_JOB`,
//! `ISAL_SHA256_MB_JOB_MGR`) is defined once here: emitted as Rust
//! constants for `multi_buffer.rs` and checked against ISA-L's headers by
//! C `_Static_assert`s compiled into the library. Nothing is executed at
//! build time, so cross-compilation works, and a layout mismatch is a
//! compile error, never a default.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ISAL: &str = "vendored/isa-l_crypto";

/// `ISAL_SHA256_JOB` (`include/isa-l_crypto/sha256_mb.h`,
/// `sha256_mb/sha256_job.asm`).
const JOB_SIZE: usize = 128;
const JOB_ALIGN: usize = 64;
const JOB_BUFFER_OFFSET: usize = 0;
const JOB_LEN_OFFSET: usize = 8;
const JOB_DIGEST_OFFSET: usize = 64;
const JOB_STATUS_OFFSET: usize = 96;
const JOB_USER_DATA_OFFSET: usize = 104;
/// `ISAL_SHA256_MB_JOB_MGR` (`sha256_mb/sha256_mb_mgr_datastruct.asm`):
/// 16-lane transposed digests and data pointers, lens, unused-lane nibbles,
/// lane jobs and the in-use count, 16-byte aligned for the `lens` vector.
const MGR_SIZE: usize = 848;
const MGR_ALIGN: usize = 16;
const MAX_LANES: usize = 16;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ISAL}");
    println!("cargo::rustc-check-cfg=cfg(isal_x86_64)");
    println!("cargo::rustc-check-cfg=cfg(isal_aarch64)");
    if std::env::var_os("CARGO_FEATURE_SHA256_MB").is_none() {
        return;
    }
    let target_arch = env("CARGO_CFG_TARGET_ARCH");
    let target_os = env("CARGO_CFG_TARGET_OS");
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let layout_check = out_dir.join("isal_layout_check.c");
    match (target_arch.as_str(), target_os.as_str()) {
        ("x86_64", _) => {
            write(&layout_check, &layout_assertions());
            write(&out_dir.join("isal_layout.rs"), &layout_constants());
            compile_x86_64_c(&layout_check);
            compile_x86_64_asm();
            audit_x86_64(&out_dir.join("libisal_sha256_mb_c.a"));
            println!("cargo:rustc-cfg=isal_x86_64");
        }
        ("aarch64", "linux") => {
            write(&layout_check, &layout_assertions());
            write(&out_dir.join("isal_layout.rs"), &layout_constants());
            compile_aarch64(&layout_check);
            println!("cargo:rustc-cfg=isal_aarch64");
        }
        // ISA-L has no multi-buffer kernel for this target, or (AArch64
        // outside Linux) its kernels use ELF-only assembler directives.
        _ => {}
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|error| panic!("{key}: {error}"))
}

fn write(path: &Path, contents: &str) {
    std::fs::write(path, contents)
        .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
}

/// The include paths and defines of ISA-L's `CMakeLists.txt`: SAFE_DATA
/// and SAFE_PARAM on, the v2.24 compatibility aliases off.
fn isal_c() -> cc::Build {
    let mut build = cc::Build::new();
    build
        .include(format!("{ISAL}/include"))
        .include(format!("{ISAL}/include/internal"))
        .include(format!("{ISAL}/include/isa-l_crypto"))
        .include(format!("{ISAL}/sha256_mb"))
        .define("SAFE_DATA", None)
        .define("SAFE_PARAM", None)
        .define("NO_COMPAT_ISAL_CRYPTO_API_2_24", None)
        .flag_if_supported("-O3")
        .warnings(false);
    build
}

/// The C sources of `SHA256_MB_X86_64_SOURCES` and
/// `SHA256_MB_BASE_SOURCES` (`cmake/sha256_mb.cmake`), plus the layout
/// assertions.
fn compile_x86_64_c(layout_check: &Path) {
    let mut build = isal_c();
    for file in [
        "sha256_mb.c",
        "sha256_ctx_base.c",
        "sha256_ctx_sse.c",
        "sha256_ctx_avx.c",
        "sha256_ctx_avx2.c",
        "sha256_ctx_avx512.c",
        "sha256_ctx_sse_ni.c",
        "sha256_ctx_avx512_ni.c",
        "sha256_mb_mgr_init_sse.c",
        "sha256_mb_mgr_init_avx2.c",
        "sha256_mb_mgr_init_avx512.c",
    ] {
        build.file(format!("{ISAL}/sha256_mb/{file}"));
    }
    build
        .file(layout_check)
        // The multibinary dispatcher reaches the per-ISA context functions
        // only through function pointers it patches at run time, so the
        // linker sees no static reference to them: link every object.
        .link_lib_modifier("+whole-archive")
        .compile("isal_sha256_mb_c");
}

/// The NASM sources of `SHA256_MB_X86_64_SOURCES`, assembled with the
/// flags of ISA-L's `CMakeLists.txt` (`-D LINUX`, `-DSAFE_DATA`,
/// `-DSAFE_PARAM`).
fn compile_x86_64_asm() {
    let mut build = nasm_rs::Build::new();
    for file in [
        "sha256_multibinary.asm",
        "sha256_mb_mgr_submit_sse.asm",
        "sha256_mb_mgr_flush_sse.asm",
        "sha256_mb_x4_sse.asm",
        "sha256_mb_mgr_submit_avx.asm",
        "sha256_mb_mgr_flush_avx.asm",
        "sha256_mb_x4_avx.asm",
        "sha256_mb_mgr_submit_avx2.asm",
        "sha256_mb_mgr_flush_avx2.asm",
        "sha256_mb_x8_avx2.asm",
        "sha256_mb_mgr_submit_avx512.asm",
        "sha256_mb_mgr_flush_avx512.asm",
        "sha256_mb_x16_avx512.asm",
        "sha256_opt_x1.asm",
        "sha256_ni_x1.asm",
        "sha256_ni_x2.asm",
        "sha256_mb_mgr_submit_sse_ni.asm",
        "sha256_mb_mgr_flush_sse_ni.asm",
        "sha256_mb_mgr_flush_avx512_ni.asm",
    ] {
        build.file(format!("{ISAL}/sha256_mb/{file}"));
    }
    build
        .include(format!("{ISAL}/include/"))
        .include(format!("{ISAL}/sha256_mb/"))
        .define("LINUX", None)
        .define("SAFE_DATA", None)
        .define("SAFE_PARAM", None)
        .compile("isal_sha256_mb_asm")
        .unwrap_or_else(|error| panic!("assembling ISA-L sha256_mb with nasm: {error}"));
    // nasm-rs emits the search path only; link every object for the same
    // reason as the C archive.
    println!("cargo:rustc-link-lib=static:+whole-archive=isal_sha256_mb_asm");
}

/// ISA-L's AArch64 scheduler (`sha256_mb_mgr_ce.c`) and the Armv8
/// crypto-extension kernels it calls (`sha256_mb_x{1,2,3,4}_ce.S`, which
/// enable `+crypto` themselves with `.arch`). The context manager and its
/// HWCAP dispatcher are not built: callers reach the scheduler directly and
/// check HWCAP themselves.
fn compile_aarch64(layout_check: &Path) {
    let mut build = isal_c();
    build.file(format!("{ISAL}/sha256_mb/aarch64/sha256_mb_mgr_ce.c"));
    for lanes in 1..=4 {
        build.file(format!("{ISAL}/sha256_mb/aarch64/sha256_mb_x{lanes}_ce.S"));
    }
    build.file(layout_check).compile("isal_sha256_mb_ce");
}

fn layout_assertions() -> String {
    format!(
        "#include <stddef.h>\n\
         #include \"sha256_mb.h\"\n\
         _Static_assert(sizeof(ISAL_SHA256_JOB) == {JOB_SIZE}, \"ISAL_SHA256_JOB size\");\n\
         _Static_assert(_Alignof(ISAL_SHA256_JOB) == {JOB_ALIGN}, \"ISAL_SHA256_JOB alignment\");\n\
         _Static_assert(offsetof(ISAL_SHA256_JOB, buffer) == {JOB_BUFFER_OFFSET}, \"job buffer\");\n\
         _Static_assert(offsetof(ISAL_SHA256_JOB, len) == {JOB_LEN_OFFSET}, \"job len\");\n\
         _Static_assert(offsetof(ISAL_SHA256_JOB, result_digest) == {JOB_DIGEST_OFFSET}, \"job digest\");\n\
         _Static_assert(offsetof(ISAL_SHA256_JOB, status) == {JOB_STATUS_OFFSET}, \"job status\");\n\
         _Static_assert(sizeof(ISAL_JOB_STS) == 4, \"job status width\");\n\
         _Static_assert(offsetof(ISAL_SHA256_JOB, user_data) == {JOB_USER_DATA_OFFSET}, \"job user data\");\n\
         _Static_assert(sizeof(ISAL_SHA256_MB_JOB_MGR) == {MGR_SIZE}, \"ISAL_SHA256_MB_JOB_MGR size\");\n\
         _Static_assert(_Alignof(ISAL_SHA256_MB_JOB_MGR) == {MGR_ALIGN}, \"ISAL_SHA256_MB_JOB_MGR alignment\");\n\
         _Static_assert(ISAL_SHA256_MAX_LANES == {MAX_LANES}, \"lane capacity\");\n"
    )
}

fn layout_constants() -> String {
    format!(
        "// The scheduler ABI defined in rekindle-hash's build.rs and checked\n\
         // against ISA-L's headers there.\n\
         pub const JOB_SIZE: usize = {JOB_SIZE};\n\
         pub const JOB_BUFFER_OFFSET: usize = {JOB_BUFFER_OFFSET};\n\
         pub const JOB_LEN_OFFSET: usize = {JOB_LEN_OFFSET};\n\
         pub const JOB_DIGEST_OFFSET: usize = {JOB_DIGEST_OFFSET};\n\
         pub const JOB_STATUS_OFFSET: usize = {JOB_STATUS_OFFSET};\n\
         pub const JOB_USER_DATA_OFFSET: usize = {JOB_USER_DATA_OFFSET};\n\
         pub const MGR_SIZE: usize = {MGR_SIZE};\n"
    )
}

/// Instructions every x86_64 target may execute: the x86-64 baseline
/// (x87, MMX, SSE, SSE2, CMOV, CX8, FXSR, SYSCALL, TSC), the hint-space
/// encodings older CPUs execute as no-ops (multi-byte NOP, `PAUSE`,
/// `ENDBR64`), and `CLFLUSH`.
fn x86_64_baseline() -> Vec<iced_x86::CpuidFeature> {
    use iced_x86::CpuidFeature as F;
    vec![
        F::INTEL8086,
        F::INTEL186,
        F::INTEL286,
        F::INTEL386,
        F::INTEL486,
        F::X64,
        F::FPU,
        F::FPU287,
        F::FPU387,
        F::MMX,
        F::SSE,
        F::SSE2,
        F::CMOV,
        F::CX8,
        F::FXSR,
        F::SYSCALL,
        F::TSC,
        F::CPUID,
        F::CLFSH,
        F::MULTIBYTENOP,
        F::PAUSE,
        F::CET_IBT,
    ]
}

/// The CPUID features a target feature enabled for this build adds,
/// by rustc's x86 feature names. Features without an entry add none.
fn x86_64_target_feature(name: &str) -> &'static [iced_x86::CpuidFeature] {
    use iced_x86::CpuidFeature as F;
    match name {
        "sse3" => &[F::SSE3],
        "ssse3" => &[F::SSSE3],
        "sse4.1" => &[F::SSE4_1],
        "sse4.2" => &[F::SSE4_2],
        "popcnt" => &[F::POPCNT],
        "cmpxchg16b" => &[F::CMPXCHG16B],
        "lzcnt" => &[F::LZCNT],
        "movbe" => &[F::MOVBE],
        "bmi1" => &[F::BMI1],
        "bmi2" => &[F::BMI2],
        "fma" => &[F::FMA],
        "f16c" => &[F::F16C],
        "avx" => &[F::AVX],
        "avx2" => &[F::AVX2],
        "avx512f" => &[F::AVX512F],
        "avx512vl" => &[F::AVX512VL],
        "avx512bw" => &[F::AVX512BW],
        "avx512cd" => &[F::AVX512CD],
        "avx512dq" => &[F::AVX512DQ],
        "aes" => &[F::AES],
        "pclmulqdq" => &[F::PCLMULQDQ],
        "sha" => &[F::SHA],
        "adx" => &[F::ADX],
        "xsave" => &[F::XSAVE],
        "rdrand" => &[F::RDRAND],
        "rdseed" => &[F::RDSEED],
        _ => &[],
    }
}

/// The ISA each compiled C source file declares via `#pragma GCC target`,
/// keyed by the stem that `cc` keeps in the archive member name
/// (`<hash>-<stem>.o`). Files without a pragma use the baseline only.
/// Derived from reading every compiled `.c` file's first 40 lines.
const PRAGMA_ISA: &[(&str, &[&str])] = &[
    ("sha256_ctx_avx", &["avx"]),
    ("sha256_ctx_avx2", &["avx2"]),
    ("sha256_ctx_avx512", &["avx2"]),
    ("sha256_ctx_avx512_ni", &["avx2"]),
    // sha256_ctx_sse, sha256_ctx_sse_ni, sha256_ctx_base,
    // sha256_mb_mgr_init_*, sha256_mb, sha256_ref,
    // isal_layout_check: no pragma, baseline only.
];

/// Extra CPUID features the source file behind `member_name` declared.
fn pragma_features(member_name: &str) -> Vec<iced_x86::CpuidFeature> {
    let mut extra = Vec::new();
    for &(stem, targets) in PRAGMA_ISA {
        // cc names members `<hex hash>-<stem>.o`
        if member_name.contains(stem) {
            for target in targets {
                extra.extend_from_slice(x86_64_target_feature(target));
            }
            break;
        }
    }
    extra
}

/// Decode every instruction of every executable section in the compiled C
/// archive and fail the build on one beyond its file's declared ISA: the
/// target baseline, plus the target features this build enables
/// (`CARGO_CFG_TARGET_FEATURE`), plus the `#pragma GCC target` the source
/// file carries. A file that uses no pragma may contain only baseline
/// instructions; a file that declares `avx2` may also use AVX and AVX2.
/// The original SIGILL came from `_sha256_mb_mgr_init_avx2` (no pragma)
/// getting zmm stores from `-mavx512f` on the command line: this audit
/// would have caught it.
fn audit_x86_64(archive_path: &Path) {
    use iced_x86::{Decoder, DecoderOptions, Instruction};
    use object::read::archive::ArchiveFile;
    use object::{Object as _, ObjectSection as _, SectionKind};

    let mut baseline_allowed = x86_64_baseline();
    for name in std::env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
    {
        baseline_allowed.extend_from_slice(x86_64_target_feature(name));
    }

    let bytes = std::fs::read(archive_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", archive_path.display()));
    let archive = ArchiveFile::parse(&*bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", archive_path.display()));
    let mut violations = String::new();
    let mut decoded = 0usize;
    for member in archive.members() {
        let member = member.unwrap_or_else(|error| panic!("archive member: {error}"));
        let name = String::from_utf8_lossy(member.name()).into_owned();
        let pragma = pragma_features(&name);
        let mut allowed = baseline_allowed.clone();
        allowed.extend_from_slice(&pragma);
        let data = member
            .data(&*bytes)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let file = object::File::parse(data).unwrap_or_else(|error| panic!("{name}: {error}"));
        for section in file.sections() {
            if section.kind() != SectionKind::Text {
                continue;
            }
            let section_name = section.name().unwrap_or("<unnamed>");
            let code = section
                .data()
                .unwrap_or_else(|error| panic!("{name} {section_name}: {error}"));
            let mut decoder = Decoder::with_ip(64, code, 0, DecoderOptions::NONE);
            let mut instruction = Instruction::default();
            while decoder.can_decode() {
                decoder.decode_out(&mut instruction);
                decoded += 1;
                if instruction.is_invalid() {
                    let _ = writeln!(
                        violations,
                        "  {name} {section_name}+{:#x}: undecodable bytes",
                        instruction.ip()
                    );
                    continue;
                }
                let beyond: Vec<_> = instruction
                    .cpuid_features()
                    .iter()
                    .filter(|feature| !allowed.contains(feature))
                    .collect();
                if !beyond.is_empty() {
                    let _ = writeln!(
                        violations,
                        "  {name} {section_name}+{:#x}: {:?} requires {beyond:?}",
                        instruction.ip(),
                        instruction.code()
                    );
                }
            }
        }
    }
    assert!(
        decoded > 0,
        "{} contains no executable code to audit",
        archive_path.display()
    );
    assert!(
        violations.is_empty(),
        "ISA-L's C was compiled with instructions beyond the source's \
         declared ISA (baseline + #pragma GCC target); the original bug \
         was -mavx512f on the command line promoting every file to \
         AVX-512:\n{violations}"
    );
}

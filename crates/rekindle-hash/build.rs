fn main() {
    #[cfg(all(feature = "sha256-mb", target_arch = "x86_64"))]
    {
        let isal = "vendored/isa-l_crypto";

        // ── C context management layer ─────────────────────────────
        //
        // ALL variant context files must be compiled because the
        // multibinary dispatcher (sha256_multibinary.asm) references
        // extern symbols from every variant: base, SSE, AVX, AVX2,
        // AVX512, SSE-NI, AVX512-NI. Missing any causes the runtime
        // dispatcher to fall back to base C (10x slower).
        let mut c_build = cc::Build::new();
        c_build
            // Top-level API (legacy + new wrappers)
            .file(format!("{isal}/sha256_mb/sha256_mb.c"))
            // All context variants — required by multibinary dispatcher.
            // sha256_ctx_base_aliases.c is excluded: it defines
            // _sha256_ctx_mgr_{init,submit,flush} which conflict with
            // the same symbols from sha256_multibinary.asm (the CPUID
            // dispatcher). The aliases file is for builds without the
            // multibinary dispatcher — we always include it.
            .file(format!("{isal}/sha256_mb/sha256_ctx_base.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_sse.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_sse_ni.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_avx.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_avx2.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_avx512.c"))
            .file(format!("{isal}/sha256_mb/sha256_ctx_avx512_ni.c"))
            // All manager init variants
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_init_sse.c"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_init_avx2.c"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_init_avx512.c"))
            // Reference implementation
            .file(format!("{isal}/sha256_mb/sha256_ref.c"))
            // Include paths
            .include(format!("{isal}/include"))
            .include(format!("{isal}/include/internal"))
            .include(format!("{isal}/include/isa-l_crypto"))
            .include(format!("{isal}/sha256_mb"))
            .flag_if_supported("-mavx2")
            .flag_if_supported("-mavx512f")
            .flag_if_supported("-msse4.1")
            .flag_if_supported("-msha")
            .flag_if_supported("-O3")
            .warnings(false)
            // Force whole-archive so the linker includes ALL object files.
            // The NASM multibinary dispatcher references _sha256_ctx_mgr_*_avx2
            // etc. at runtime via patched function pointers — the linker can't
            // see these references statically and would otherwise discard the
            // AVX2 context files, causing the dispatcher to fall back to base C.
            .link_lib_modifier("+whole-archive")
            .compile("isal_sha256_mb_c");

        // ── NASM assembly SIMD kernels ─────────────────────────────
        //
        // ALL NASM assembly files for every dispatch variant must be
        // compiled. The multibinary dispatcher selects at runtime via
        // CPUID. Missing assembly = linker resolves but runtime falls
        // back to base C.
        nasm_rs::Build::new()
            // Multibinary dispatcher (CPUID-based function pointer init)
            .file(format!("{isal}/sha256_mb/sha256_multibinary.asm"))
            // SSE kernels
            .file(format!("{isal}/sha256_mb/sha256_mb_x4_sse.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_submit_sse.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_sse.asm"))
            // AVX kernels
            .file(format!("{isal}/sha256_mb/sha256_mb_x4_avx.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_submit_avx.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_avx.asm"))
            // AVX2 kernels (8-way — the primary performance target)
            .file(format!("{isal}/sha256_mb/sha256_mb_x8_avx2.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_submit_avx2.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_avx2.asm"))
            // AVX512 kernels (16-way)
            .file(format!("{isal}/sha256_mb/sha256_mb_x16_avx512.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_submit_avx512.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_avx512.asm"))
            // SHA-NI kernels (hardware SHA instructions)
            .file(format!("{isal}/sha256_mb/sha256_ni_x1.asm"))
            .file(format!("{isal}/sha256_mb/sha256_ni_x2.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_submit_sse_ni.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_sse_ni.asm"))
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_flush_avx512_ni.asm"))
            // Datastruct helpers
            .file(format!("{isal}/sha256_mb/sha256_mb_mgr_datastruct.asm"))
            .file(format!("{isal}/sha256_mb/sha256_job.asm"))
            // Opt single-buffer (used by some paths)
            .file(format!("{isal}/sha256_mb/sha256_opt_x1.asm"))
            // Include paths
            .include(format!("{isal}/include/"))
            .include(format!("{isal}/sha256_mb/"))
            .define("__linux__", None)
            .compile("isal_sha256_mb_asm")
            .expect("NASM assembly compilation failed for ISA-L sha256_mb. \
                     Install nasm: `nix develop` or `apt install nasm`.");

        // nasm-rs only emits cargo:rustc-link-search, NOT cargo:rustc-link-lib.
        // We must explicitly link the ASM archive with +whole-archive so ALL
        // symbols (AVX2 submit/flush kernels, multibinary dispatcher) are
        // included regardless of whether Rust directly references them.
        println!("cargo:rustc-link-lib=static:+whole-archive=isal_sha256_mb_asm");

        // ── Sizeof probe ───────────────────────────────────────────
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let probe_src = format!("{out_dir}/isal_sizeof_probe.c");
        std::fs::write(&probe_src, r#"
#include <isa-l_crypto/sha256_mb.h>
#include <stddef.h>
#include <stdio.h>
int main(void) {
    printf("MGR_SIZE=%zu\n", sizeof(SHA256_HASH_CTX_MGR));
    printf("CTX_SIZE=%zu\n", sizeof(SHA256_HASH_CTX));
    printf("DIGEST_OFFSET=%zu\n", offsetof(SHA256_HASH_CTX, job.result_digest));
    printf("JOB_SIZE=%zu\n", sizeof(ISAL_SHA256_JOB));
    printf("JOB_MGR_SIZE=%zu\n", sizeof(ISAL_SHA256_MB_JOB_MGR));
    printf("JOB_BUFFER_OFFSET=%zu\n", offsetof(ISAL_SHA256_JOB, buffer));
    printf("JOB_LEN_OFFSET=%zu\n", offsetof(ISAL_SHA256_JOB, len));
    printf("JOB_DIGEST_OFFSET=%zu\n", offsetof(ISAL_SHA256_JOB, result_digest));
    printf("JOB_STATUS_OFFSET=%zu\n", offsetof(ISAL_SHA256_JOB, status));
    return 0;
}
"#).expect("failed to write sizeof probe source");

        let probe_bin = format!("{out_dir}/isal_sizeof_probe");
        let c_lib = format!("{out_dir}/libisal_sha256_mb_c.a");
        let asm_lib = format!("{out_dir}/libisal_sha256_mb_asm.a");
        let compile_status = std::process::Command::new(
            std::env::var("CC").unwrap_or_else(|_| "cc".to_string())
        )
            .arg(&probe_src)
            .arg("-o").arg(&probe_bin)
            .arg(format!("-I{isal}/include"))
            .arg(format!("-I{isal}/include/internal"))
            .arg(format!("-I{isal}/include/isa-l_crypto"))
            .arg(&c_lib)
            .arg(&asm_lib)
            .status();

        match compile_status {
            Ok(status) if status.success() => {
                let output = std::process::Command::new(&probe_bin)
                    .output()
                    .expect("failed to run sizeof probe");

                let stdout = String::from_utf8(output.stdout)
                    .expect("sizeof probe produced non-UTF8 output");

                let mut job_size = 0usize;
                let mut job_mgr_size = 0usize;
                let mut job_buffer_offset = 0usize;
                let mut job_len_offset = 0usize;
                let mut job_digest_offset = 0usize;
                let mut job_status_offset = 0usize;

                for line in stdout.lines() {
                    if let Some(val) = line.strip_prefix("JOB_SIZE=") {
                        job_size = val.parse().expect("invalid JOB_SIZE");
                    } else if let Some(val) = line.strip_prefix("JOB_MGR_SIZE=") {
                        job_mgr_size = val.parse().expect("invalid JOB_MGR_SIZE");
                    } else if let Some(val) = line.strip_prefix("JOB_BUFFER_OFFSET=") {
                        job_buffer_offset = val.parse().expect("invalid JOB_BUFFER_OFFSET");
                    } else if let Some(val) = line.strip_prefix("JOB_LEN_OFFSET=") {
                        job_len_offset = val.parse().expect("invalid JOB_LEN_OFFSET");
                    } else if let Some(val) = line.strip_prefix("JOB_DIGEST_OFFSET=") {
                        job_digest_offset = val.parse().expect("invalid JOB_DIGEST_OFFSET");
                    } else if let Some(val) = line.strip_prefix("JOB_STATUS_OFFSET=") {
                        job_status_offset = val.parse().expect("invalid JOB_STATUS_OFFSET");
                    }
                }

                assert!(job_size > 0, "sizeof probe: JOB_SIZE was 0 or missing");
                assert!(job_mgr_size > 0, "sizeof probe: JOB_MGR_SIZE was 0 or missing");

                let generated = format!(
                    "/// Probed at build time from ISA-L headers.\n\
                     pub const PROBED_JOB_SIZE: usize = {job_size};\n\
                     pub const PROBED_JOB_MGR_SIZE: usize = {job_mgr_size};\n\
                     pub const PROBED_JOB_BUFFER_OFFSET: usize = {job_buffer_offset};\n\
                     pub const PROBED_JOB_LEN_OFFSET: usize = {job_len_offset};\n\
                     pub const PROBED_JOB_DIGEST_OFFSET: usize = {job_digest_offset};\n\
                     pub const PROBED_JOB_STATUS_OFFSET: usize = {job_status_offset};\n"
                );
                std::fs::write(format!("{out_dir}/isal_sizes.rs"), generated)
                    .expect("failed to write isal_sizes.rs");
            }
            _ => {
                println!("cargo:warning=sizeof probe failed — using conservative default sizes");
                let generated =
                    "/// Conservative defaults — sizeof probe did not run.\n\
                     pub const PROBED_JOB_SIZE: usize = 128;\n\
                     pub const PROBED_JOB_MGR_SIZE: usize = 4096;\n\
                     pub const PROBED_JOB_BUFFER_OFFSET: usize = 0;\n\
                     pub const PROBED_JOB_LEN_OFFSET: usize = 8;\n\
                     pub const PROBED_JOB_DIGEST_OFFSET: usize = 64;\n\
                     pub const PROBED_JOB_STATUS_OFFSET: usize = 96;\n";
                std::fs::write(format!("{out_dir}/isal_sizes.rs"), generated)
                    .expect("failed to write isal_sizes.rs");
            }
        }
    }
}

#!/usr/bin/env bash
# rekindle-hash ISA matrix: run the library tests on emulated x86_64 CPUs.
#
# Each CPU model runs the full test suite under qemu-x86_64 (cargo's target
# runner) with the kernel set that model must support. Every supported
# kernel is checked against single-buffer SHA-256 on the emulated CPU, so
# an instruction beyond a model's ISA anywhere on a kernel's path (the
# compiled C included) faults there, and a model that gains or loses a
# kernel fails `supported_kernels_match_the_expected_set_when_one_is_given`.
# A regression is caught on any build host, whatever its own CPU.
#
# Usage: bash crates/rekindle-hash/scripts/isa-matrix.sh   (in `nix develop`)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
MANIFEST="$WORKSPACE_DIR/Cargo.toml"

command -v qemu-x86_64 >/dev/null || {
    echo "qemu-x86_64 not found: run inside rekindle's nix develop shell" >&2
    exit 1
}
[[ "$(uname -m)" == "x86_64" ]] || {
    echo "the ISA matrix runs x86_64 test binaries; this host is $(uname -m)" >&2
    exit 1
}

# CPU model, then the kernels it supports in Kernel::ALL order.
#
# qemu TCG (user-mode emulation) does not implement AVX-512: it strips
# those CPUID bits even when the model advertises them. SHA-NI (SHA
# extensions over SSE registers) IS emulated by TCG, so Denverton and
# Icelake-Server expose it. The expected kernel sets here reflect what
# qemu TCG actually exposes, verified by running each model and reading
# its `supported()` output. AVX-512 kernels are tested only on native
# hardware. The matrix's value is catching baseline, SSE, AVX, AVX2 and
# SHA-NI regressions on any build host.
MODELS=(
    "qemu64|"                                                   # x86-64 baseline: no SSE4.2
    "Nehalem|sse"                                               # SSE4.2
    "Denverton|sse,sse_ni"                                      # SSE4.2 and SHA-NI, no AVX
    "SandyBridge|sse,avx"                                       # AVX
    "Haswell|sse,avx,avx2"                                      # AVX2
    "Skylake-Server|sse,avx,avx2"                               # AVX-512 stripped by TCG
    "Icelake-Server|sse,avx,avx2"                               # AVX-512 stripped; SHA-NI not in this qemu model's CPUID
)

# One build serves every model.
cargo test --manifest-path "$MANIFEST" -p rekindle-hash --lib --no-run

status=0
for entry in "${MODELS[@]}"; do
    model="${entry%%|*}"
    expected="${entry#*|}"
    echo "== $model: expecting [${expected}]"
    if CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="qemu-x86_64 -cpu $model" \
        REKINDLE_HASH_EXPECT_KERNELS="$expected" \
        cargo test --manifest-path "$MANIFEST" -p rekindle-hash --lib; then
        echo "== $model: pass"
    else
        model_status=$?
        echo "== $model: FAIL (exit $model_status)"
        if [[ $status -eq 0 ]]; then
            status=$model_status
        fi
    fi
done
exit "$status"

#!/usr/bin/env bash
# Linux dev mode with captured logs: plain `pnpm tauri dev` piped
# through tee so the session's tracing output lands in a file other
# tooling can tail (target/debug/rekindle-dev.log) instead of living
# only in the launching terminal's scrollback.
#
# This is a LOG-CAPTURE pipe, not a launch wrapper — Linux launch
# quirks stay in-binary (platform.rs::linux_display_setup) and `tauri
# dev`'s Rust auto-rebuild is untouched; see
# docs/contributor/development.md.
#
# Usage:  ./scripts/linux-dev.sh        (or: pnpm dev:linux)
#   - log: target/debug/rekindle-dev.log (truncated each run)
#   - RUST_LOG is honored when set; the default mirrors the in-binary
#     filter plus the media-plane debug targets — the dropped-frame
#     paths worth a log file in the first place log at debug
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LOG="$ROOT/target/debug/rekindle-dev.log"

export RUST_LOG="${RUST_LOG:-info,veilid_api=warn,veilid_core=warn,rekindle_video=debug,rekindle_voice=debug}"

# A still-running dev instance must exit first — the single-instance
# plugin turns a second launch into a focus-the-old no-op. Take the old
# `tauri dev` CLI down too so two watchers don't fight over
# target/debug. (`pnpm dev`, the beforeDevCommand, clears a stale Vite
# on :1420 by itself.)
for pattern in "$ROOT/.*tauri\.js dev" "$ROOT/target/debug/rekindle"; do
    if pgrep -f "$pattern" >/dev/null; then
        echo "→ stopping running dev processes ($pattern)"
        pkill -f "$pattern" || true
    fi
done
sleep 1

mkdir -p "${LOG%/*}"
: > "$LOG"
echo "→ live log: $LOG"
echo "→ RUST_LOG: $RUST_LOG"
cd "$ROOT"
pnpm tauri dev 2>&1 | tee "$LOG"

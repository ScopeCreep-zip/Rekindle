#!/usr/bin/env bash
# Rekindle — Build wrapper (macOS/Linux)
# Usage: bash scripts/build.sh
#
# NOTE: the Konductor Nix flake is the CANONICAL dev environment —
#   nix develop .#frontend
# provides every dependency below with pinned versions. This script is
# the NON-NIX FALLBACK (contributors without nix, CI images); when the
# two disagree, the flake wins. Keep dependency changes in sync with it.
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"

cd "$PROJECT_DIR"

# ── Prerequisites ────────────────────────────────────────────────────
command -v rustc &>/dev/null || error "Rust not found — run: bash scripts/setup-$(uname -s | tr '[:upper:]' '[:lower:]' | sed 's/darwin/macos/').sh"
command -v node &>/dev/null || error "Node.js not found — run the setup script first"
command -v pnpm &>/dev/null || error "pnpm not found — run: corepack enable"
command -v capnp &>/dev/null || error "Cap'n Proto compiler not found — run the setup script first"
command -v cmake &>/dev/null || error "CMake not found — run the setup script first"

info "Rust:  $(rustc --version)"
info "Node:  $(node --version)"
info "pnpm:  $(pnpm --version)"
info "capnp: $(capnp --version 2>&1 | head -1)"
info "cmake: $(cmake --version | head -1)"

# ── Frontend dependencies ───────────────────────────────────────────
if [[ ! -d node_modules ]]; then
    warn "Installing frontend dependencies..."
    pnpm install
fi

# ── Build ────────────────────────────────────────────────────────────
# pnpm tauri build triggers beforeBuildCommand ("pnpm build") which runs
# copy-sidecar.mjs --release to build rekindle-server and copy it to
# src-tauri/binaries/ with the target-triple suffix. Tauri's externalBin
# config then includes it in the production bundle automatically.
info "Building Rekindle..."
pnpm tauri build

# ── Report artifacts ─────────────────────────────────────────────────
BUNDLE_DIR="src-tauri/target/release/bundle"

# Print "  <Label>: <dir>/" then each matching file indented under it.
#
# Globbing rather than `ls | sed`: a bundle filename carrying a space (the
# product name is configurable) would be split across two lines by `ls`, and
# the pipeline's exit status was `sed`'s, so an unreadable bundle directory
# still reported success. An unmatched glob stays literal, which `-e` filters.
report_bundle() {
    local label="$1" subdir="$2" ext="$3" file
    [[ -d "$BUNDLE_DIR/$subdir" ]] || return 0
    echo "  ${label}: $BUNDLE_DIR/$subdir/"
    for file in "$BUNDLE_DIR/$subdir"/*."$ext"; do
        [[ -e "$file" ]] || continue
        echo "    $file"
    done
}

echo ""
info "Build complete! Artifacts:"
case "$(uname -s)" in
    Darwin)
        report_bundle "DMG" dmg dmg
        # .app is a bundle directory, not a file — name the folder itself.
        if [[ -d "$BUNDLE_DIR/macos" ]]; then
            echo "  App: $BUNDLE_DIR/macos/"
        fi
        ;;
    Linux)
        report_bundle "AppImage" appimage AppImage
        report_bundle "Deb" deb deb
        ;;
esac
echo ""

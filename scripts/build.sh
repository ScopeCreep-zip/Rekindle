#!/usr/bin/env bash
# Rekindle — Build wrapper (macOS/Linux)
# Usage: bash scripts/build.sh
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
error() { echo -e "${RED}[x]${NC} $*"; exit 1; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_DIR"

# ── Prerequisites ────────────────────────────────────────────────────
command -v rustc  &>/dev/null || error "Rust not found — run: bash scripts/setup-$(uname -s | tr '[:upper:]' '[:lower:]' | sed 's/darwin/macos/').sh"
command -v node   &>/dev/null || error "Node.js not found — run the setup script first"
command -v pnpm   &>/dev/null || error "pnpm not found — run: corepack enable"
command -v capnp  &>/dev/null || error "Cap'n Proto compiler not found — run the setup script first"
command -v cmake  &>/dev/null || error "CMake not found — run the setup script first"

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
info "Building Rekindle..."
pnpm tauri build

# ── Report artifacts ─────────────────────────────────────────────────
echo ""
info "Build complete! Artifacts:"
case "$(uname -s)" in
    Darwin)
        BUNDLE_DIR="src-tauri/target/release/bundle"
        if [[ -d "$BUNDLE_DIR/dmg" ]]; then
            echo "  DMG: $BUNDLE_DIR/dmg/"
            ls "$BUNDLE_DIR/dmg/"*.dmg 2>/dev/null | sed 's/^/    /'
        fi
        if [[ -d "$BUNDLE_DIR/macos" ]]; then
            echo "  App: $BUNDLE_DIR/macos/"
        fi
        ;;
    Linux)
        BUNDLE_DIR="src-tauri/target/release/bundle"
        if [[ -d "$BUNDLE_DIR/appimage" ]]; then
            echo "  AppImage: $BUNDLE_DIR/appimage/"
            ls "$BUNDLE_DIR/appimage/"*.AppImage 2>/dev/null | sed 's/^/    /'
        fi
        if [[ -d "$BUNDLE_DIR/deb" ]]; then
            echo "  Deb: $BUNDLE_DIR/deb/"
            ls "$BUNDLE_DIR/deb/"*.deb 2>/dev/null | sed 's/^/    /'
        fi
        ;;
esac
echo ""

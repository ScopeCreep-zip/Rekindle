#!/usr/bin/env bash
# shellcheck shell=bash
#
# Rekindle — shared helpers for the bash entry points in scripts/.
#
# build.sh, setup-linux.sh and setup-macos.sh each carried a byte-identical
# copy of the colour palette and the info/warn/error trio, and the two setup
# scripts additionally duplicated the repo-root resolution, the rustup block,
# the corepack/pnpm check, the node_modules install and the closing summary —
# about 65 of their 382 combined lines. They live here so a change to the
# toolchain bootstrap lands in every script at once instead of in whichever
# one the author happened to open.
#
# setup-windows.ps1 is deliberately NOT covered: PowerShell has no `source`,
# and mirroring these helpers in a second language would recreate the drift
# this file exists to prevent.
#
# Usage, from any script directly under scripts/:
#
#     # shellcheck source=scripts/lib/common.sh
#     source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"
#
# Sourced, never executed — it defines helpers and sets PROJECT_DIR, and
# performs no work of its own. Callers own `set -euo pipefail`.

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info() { echo -e "${GREEN}[+]${NC} $*"; }
warn() { echo -e "${YELLOW}[!]${NC} $*"; }
error() {
    echo -e "${RED}[x]${NC} $*"
    exit 1
}

# Repo root, resolved from this file rather than from the caller — every
# script lives exactly one level above lib/, so this is the same answer the
# per-script `SCRIPT_DIR`/`PROJECT_DIR` pairs computed, minus the chance of
# one of them counting `dirname` levels wrong.
REKINDLE_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT_DIR="$(dirname "$REKINDLE_LIB_DIR")"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Install rustup + a stable toolchain, or update an existing one.
#
# `rustup update` is allowed to fail (`|| true`): a distro-packaged rustup
# managed by the system package manager refuses self-directed updates, and
# that is not a reason to abort a setup run on an already-working toolchain.
ensure_rust() {
    if command -v rustup &>/dev/null; then
        info "Rust already installed ($(rustc --version))"
        rustup update stable --no-self-update 2>/dev/null || true
    else
        warn "Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        # shellcheck source=/dev/null
        source "$HOME/.cargo/env"
        info "Rust installed ($(rustc --version))"
    fi
}

# Turn on corepack so `pnpm` resolves to the version pinned by
# package.json's `packageManager` field, then report what we ended up with.
#
# Neither half is fatal: a Node install without corepack, or a corepack that
# cannot write its shims without sudo, both leave the contributor able to
# fall back to `npm install -g pnpm`. Warn and continue rather than abort a
# setup that has already installed the system packages.
ensure_pnpm() {
    if command -v corepack &>/dev/null; then
        corepack enable 2>/dev/null || warn "corepack enable failed — you may need: sudo corepack enable"
    else
        warn "corepack not found — install Node.js 22+ first"
    fi

    if command -v pnpm &>/dev/null; then
        info "pnpm available ($(pnpm --version))"
    else
        warn "pnpm not found after corepack enable — try: npm install -g pnpm"
    fi
}

# Populate node_modules on a fresh clone. An existing tree is left alone —
# re-running install would be the contributor's call, not the setup script's.
install_frontend_deps() {
    if [[ -d "$PROJECT_DIR/node_modules" ]]; then
        info "node_modules exists — run 'pnpm install' manually if needed"
    else
        warn "Running pnpm install..."
        (cd "$PROJECT_DIR" && pnpm install)
        info "Frontend dependencies installed"
    fi
}

# Closing banner for the setup scripts. $1 is the platform name as it should
# read in the headline, e.g. "macOS" or "Linux".
print_setup_summary() {
    local platform="$1"
    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Rekindle ${platform} setup complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo "  Next steps:"
    echo "    cd $(basename "$PROJECT_DIR")"
    echo "    pnpm tauri dev      # Start development"
    echo "    pnpm tauri build    # Build for distribution"
    echo ""
}

#!/usr/bin/env bash
# Rekindle — macOS developer setup (Intel + Apple Silicon)
# Usage: bash scripts/setup-macos.sh
#
# NOTE: the Konductor Nix flake is the CANONICAL dev environment —
#   nix develop .#frontend
# provides every dependency below with pinned versions. This script is
# the NON-NIX FALLBACK (contributors without nix, CI images); when the
# two disagree, the flake wins. Keep dependency changes in sync with it.
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"

# ── Xcode CLI Tools ──────────────────────────────────────────────────
if xcode-select -p &>/dev/null; then
    info "Xcode CLI tools already installed"
else
    warn "Installing Xcode CLI tools..."
    xcode-select --install
    echo "    Waiting for Xcode CLI tools installer to finish..."
    until xcode-select -p &>/dev/null; do sleep 5; done
    info "Xcode CLI tools installed"
fi

# ── Homebrew ─────────────────────────────────────────────────────────
if command -v brew &>/dev/null; then
    info "Homebrew already installed"
else
    warn "Installing Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    # Add to PATH for the rest of this script
    if [[ -f /opt/homebrew/bin/brew ]]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [[ -f /usr/local/bin/brew ]]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi
    info "Homebrew installed"
fi

# ── Brew packages ────────────────────────────────────────────────────
BREW_PACKAGES=(capnp opus cmake pkg-config)
for pkg in "${BREW_PACKAGES[@]}"; do
    if brew list "$pkg" &>/dev/null; then
        info "$pkg already installed"
    else
        warn "Installing $pkg..."
        brew install "$pkg"
        info "$pkg installed"
    fi
done

# ── Rust ─────────────────────────────────────────────────────────────
ensure_rust

# Add both macOS targets
rustup target add aarch64-apple-darwin 2>/dev/null || true
rustup target add x86_64-apple-darwin 2>/dev/null || true

# ── Node.js ──────────────────────────────────────────────────────────
if command -v node &>/dev/null; then
    NODE_VER=$(node --version)
    info "Node.js already installed ($NODE_VER)"
else
    warn "Installing Node.js 22 LTS via Homebrew..."
    brew install node@22
    info "Node.js installed ($(node --version))"
fi

# ── pnpm via corepack ───────────────────────────────────────────────
ensure_pnpm

# ── Install frontend dependencies ───────────────────────────────────
install_frontend_deps

# ── Summary ──────────────────────────────────────────────────────────
print_setup_summary "macOS"

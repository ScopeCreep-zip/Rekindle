#!/usr/bin/env bash
# Rekindle — Linux developer setup (Debian/Ubuntu/Pop!_OS, Fedora, Arch)
# Usage: bash scripts/setup-linux.sh
#
# NOTE: the Konductor Nix flake is the CANONICAL dev environment —
#   nix develop .#frontend
# provides every dependency below with pinned versions. This script is
# the NON-NIX FALLBACK (contributors without nix, CI images); when the
# two disagree, the flake wins. Keep dependency changes in sync with it.
set -euo pipefail

# shellcheck source=lib/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"

# ── Distro detection ────────────────────────────────────────────────
if [[ ! -f /etc/os-release ]]; then
    error "Cannot detect distro — /etc/os-release not found"
fi

# /etc/os-release is a runtime OS file, not a repo file — shellcheck cannot
# read it at lint time and must not try. The `ID` / `ID_LIKE` / `PRETTY_NAME`
# variables it sets are the ones consumed just below.
# shellcheck source=/dev/null
source /etc/os-release

case "${ID:-}" in
    ubuntu | debian | pop | linuxmint | elementary | zorin)
        DISTRO_FAMILY="debian"
        ;;
    fedora)
        DISTRO_FAMILY="fedora"
        ;;
    arch | manjaro | endeavouros)
        DISTRO_FAMILY="arch"
        ;;
    *)
        # Check ID_LIKE for derivatives
        case "${ID_LIKE:-}" in
            *debian* | *ubuntu*)
                DISTRO_FAMILY="debian"
                ;;
            *fedora* | *rhel*)
                DISTRO_FAMILY="fedora"
                ;;
            *arch*)
                DISTRO_FAMILY="arch"
                ;;
            *)
                error "Unsupported distro: ${PRETTY_NAME:-$ID}. Supported: Debian/Ubuntu/Pop!_OS, Fedora, Arch"
                ;;
        esac
        ;;
esac

info "Detected distro family: $DISTRO_FAMILY (${PRETTY_NAME:-$ID})"

# ── System packages ─────────────────────────────────────────────────
case "$DISTRO_FAMILY" in
    debian)
        info "Installing packages via apt..."
        sudo apt-get update
        sudo apt-get install -y \
            build-essential \
            pkg-config \
            cmake \
            curl \
            wget \
            file \
            libwebkit2gtk-4.1-dev \
            libssl-dev \
            libayatana-appindicator3-dev \
            librsvg2-dev \
            libxdo-dev \
            libasound2-dev \
            libopus-dev \
            capnproto \
            patchelf
        ;;
    fedora)
        info "Installing packages via dnf..."
        sudo dnf groupinstall -y "C Development Tools and Libraries"
        sudo dnf install -y \
            pkg-config \
            cmake \
            curl \
            wget \
            file \
            webkit2gtk4.1-devel \
            openssl-devel \
            libappindicator-gtk3-devel \
            librsvg2-devel \
            libxdo-devel \
            alsa-lib-devel \
            opus-devel \
            capnproto \
            patchelf
        ;;
    arch)
        info "Installing packages via pacman..."
        sudo pacman -Syu --needed --noconfirm \
            base-devel \
            pkg-config \
            cmake \
            curl \
            wget \
            file \
            webkit2gtk-4.1 \
            openssl \
            libappindicator-gtk3 \
            librsvg \
            xdotool \
            alsa-lib \
            opus \
            capnproto \
            patchelf
        ;;
esac

info "System packages installed"

# ── Rust ─────────────────────────────────────────────────────────────
ensure_rust

# ── Node.js ──────────────────────────────────────────────────────────
if command -v node &>/dev/null; then
    NODE_VER=$(node --version)
    info "Node.js already installed ($NODE_VER)"
else
    warn "Installing Node.js 22 LTS via fnm..."
    if ! command -v fnm &>/dev/null; then
        curl -fsSL https://fnm.vercel.app/install | bash
        export PATH="$HOME/.local/share/fnm:$PATH"
        eval "$(fnm env)"
    fi
    fnm install 22
    fnm use 22
    info "Node.js installed ($(node --version))"
fi

# ── pnpm via corepack ───────────────────────────────────────────────
ensure_pnpm

# ── Install frontend dependencies ───────────────────────────────────
install_frontend_deps

# ── Summary ──────────────────────────────────────────────────────────
print_setup_summary "Linux"

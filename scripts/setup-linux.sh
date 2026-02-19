#!/usr/bin/env bash
# Rekindle — Linux developer setup (Debian/Ubuntu/Pop!_OS, Fedora, Arch)
# Usage: bash scripts/setup-linux.sh
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
error() { echo -e "${RED}[x]${NC} $*"; exit 1; }

# ── Distro detection ────────────────────────────────────────────────
if [[ ! -f /etc/os-release ]]; then
    error "Cannot detect distro — /etc/os-release not found"
fi

source /etc/os-release

case "${ID:-}" in
    ubuntu|debian|pop|linuxmint|elementary|zorin)
        DISTRO_FAMILY="debian"
        ;;
    fedora)
        DISTRO_FAMILY="fedora"
        ;;
    arch|manjaro|endeavouros)
        DISTRO_FAMILY="arch"
        ;;
    *)
        # Check ID_LIKE for derivatives
        case "${ID_LIKE:-}" in
            *debian*|*ubuntu*)
                DISTRO_FAMILY="debian"
                ;;
            *fedora*|*rhel*)
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
if command -v rustup &>/dev/null; then
    info "Rust already installed ($(rustc --version))"
    rustup update stable --no-self-update 2>/dev/null || true
else
    warn "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    info "Rust installed ($(rustc --version))"
fi

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

# ── Install frontend dependencies ───────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

if [[ -d "$PROJECT_DIR/node_modules" ]]; then
    info "node_modules exists — run 'pnpm install' manually if needed"
else
    warn "Running pnpm install..."
    (cd "$PROJECT_DIR" && pnpm install)
    info "Frontend dependencies installed"
fi

# ── Summary ──────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  Rekindle Linux setup complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "  Next steps:"
echo "    cd $(basename "$PROJECT_DIR")"
echo "    pnpm tauri dev      # Start development"
echo "    pnpm tauri build    # Build for distribution"
echo ""

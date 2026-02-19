#!/usr/bin/env bash
# Rekindle — macOS developer setup (Intel + Apple Silicon)
# Usage: bash scripts/setup-macos.sh
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
error() { echo -e "${RED}[x]${NC} $*"; exit 1; }

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
if command -v rustup &>/dev/null; then
    info "Rust already installed ($(rustc --version))"
    rustup update stable --no-self-update 2>/dev/null || true
else
    warn "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    info "Rust installed ($(rustc --version))"
fi

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
echo -e "${GREEN}  Rekindle macOS setup complete!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "  Next steps:"
echo "    cd $(basename "$PROJECT_DIR")"
echo "    pnpm tauri dev      # Start development"
echo "    pnpm tauri build    # Build for distribution"
echo ""

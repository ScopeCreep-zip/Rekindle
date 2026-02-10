# Development Environment Setup

## Option A: Konductor (Recommended)

[Konductor](https://github.com/braincraftio/konductor) is a Nix flake that provides reproducible
development environments. Its `frontend` devshell includes everything needed to build Tauri 2 apps.

### Prerequisites

- [Nix package manager](https://nixos.org/download/) with flakes enabled

```bash
# Enable flakes (if not already)
echo "experimental-features = nix-command flakes" >> ~/.config/nix/nix.conf
```

### Enter the Dev Environment

```bash
# Clone the repo
git clone https://github.com/ScopeCreep-zip/Rekindle.git
cd Rekindle

# Enter the Konductor frontend shell
nix develop .#frontend
```

This provides:
- **Rust 1.92+** with cargo, clippy, rust-analyzer
- **Node.js 22** with pnpm
- **Tauri 2 system deps**: GTK, WebKitGTK, OpenSSL
- **Playwright** with bundled browsers for E2E testing
- **13 linters + 8 formatters** with hermetic configs
- **Dev tools**: Neovim, Tmux, ripgrep, fzf, lazygit

### Using Konductor as a Flake Input

To integrate Konductor into the project's own `flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    konductor.url = "github:braincraftio/konductor";
  };

  outputs = { self, nixpkgs, konductor, ... }: {
    devShells.x86_64-linux.default = konductor.devShells.x86_64-linux.frontend;
  };
}
```

### Platform Note

The Konductor `frontend` shell is **Linux-only** (x86_64-linux) due to GTK/WebKitGTK dependencies.
For macOS development, use Option B below.

---

## Option B: Manual Setup

### Rust

```bash
# Install rustup
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ensure stable toolchain
rustup default stable
rustup update
```

### Node.js

```bash
# Install Node.js 20+ (via nvm, fnm, or system package manager)
# Then install pnpm
npm install -g pnpm
```

### Tauri 2 System Dependencies

Follow the official guide for your platform:
https://v2.tauri.app/start/prerequisites/

**Linux (Debian/Ubuntu):**
```bash
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

**macOS:**
```bash
xcode-select --install
```

**Windows:**
- Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
- Install [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- Install [Rust](https://www.rust-lang.org/tools/install)

---

## Project Setup

```bash
# Install frontend dependencies
pnpm install

# Run in development mode (hot-reload for both frontend and Rust)
pnpm tauri dev

# Build for production
pnpm tauri build

# Run protocol crate tests
cargo test -p rekindle-protocol

# Run all tests
cargo test --workspace
```

## Reverse Engineering Tools

These are optional, for analyzing the `xf1re_installer.exe`:

```bash
# Unpack NSIS installer
brew install p7zip    # macOS
# sudo apt install p7zip-full  # Linux
7z x xf1re_installer.exe -o./unpacked

# Static analysis
brew install radare2  # or rizin
# Ghidra: download from https://ghidra-sre.org/

# Inspect binaries
file ./unpacked/*
strings ./unpacked/<binary> | grep -i "xfire\|UA01\|UltimateArena"
r2 -A ./unpacked/<binary>
```

## IDE Setup

### VS Code
- Install the Tauri extension: `tauri-apps.tauri-vscode`
- Install rust-analyzer: `rust-lang.rust-analyzer`
- The Konductor shell includes Neovim with 10 LSPs pre-configured

### Recommended Extensions
- Tauri (`tauri-apps.tauri-vscode`)
- rust-analyzer (`rust-lang.rust-analyzer`)
- ESLint / Biome (frontend linting)
- Prettier (frontend formatting)

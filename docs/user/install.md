# Installing Rekindle

Rekindle is a Tauri 2 desktop application. We target three operating
systems with the standard distribution format for each:

| OS | Format | Notes |
|----|--------|-------|
| Windows | NSIS installer (`.exe`) | Windows 10 1903+ / Windows 11; uses the system WebView2 runtime |
| macOS | DMG (`.dmg`) | macOS 11 Big Sur or later, both Intel and Apple Silicon |
| Linux | AppImage and `.deb` | x86_64; GTK 3 + WebKitGTK 4.1 required |

> Pre-1.0 status: until the first tagged release lands, we do not
> publish pre-built artifacts. The only install path today is to
> build from source. See
> [`../contributor/development.md`](../contributor/development.md)
> for the full development setup, or follow the per-OS source-build
> notes below.

## Pre-built artifacts (post-1.0)

Once releases ship, install paths will be:

### Windows

1. Download `Rekindle-x.y.z-setup.exe` from the
   [Releases page](https://github.com/ScopeCreep-zip/Rekindle/releases).
2. Run the installer. Windows SmartScreen may warn that the
   publisher is new — click **More info** → **Run anyway** until the
   first signed release.
3. The installer registers the `rekindle://` URL scheme so invite
   links work from a browser.

### macOS

1. Download `Rekindle-x.y.z.dmg` from the Releases page.
2. Open the DMG and drag **Rekindle.app** into **Applications**.
3. The first launch needs **Right-click → Open** because the app is
   not yet notarised by Apple.

### Linux

Two formats are published:

**AppImage** (most distros):

```sh
chmod +x Rekindle-x.y.z.AppImage
./Rekindle-x.y.z.AppImage
```

**Debian / Ubuntu:**

```sh
sudo dpkg -i rekindle_x.y.z_amd64.deb
sudo apt --fix-broken install   # in case dependencies are missing
```

WebKitGTK 4.1 must be available. On Ubuntu 24.04 it ships in the
default repos; on older versions you may need to enable the
`universe` repo.

## Building from source

If you are running pre-1.0 or want to track the bleeding edge, build
from source:

### Prerequisites

| Component | Minimum | Notes |
|-----------|---------|-------|
| Rust | 1.92 | Use [`rustup`](https://rustup.rs/) |
| Node.js | 22 | Use [`nvm`](https://github.com/nvm-sh/nvm) or [`fnm`](https://github.com/Schniz/fnm) |
| pnpm | 9+ | `npm install -g pnpm` |
| OS deps | varies | See per-OS section below |

The project ships a Nix flake (`flake.nix`) that provides every
dependency hermetically. If you have Nix with flakes enabled:

```sh
nix develop .#frontend
pnpm install
pnpm tauri dev
```

This is the recommended path for contributors because every
dependency version is pinned.

### Without Nix

#### Linux (Debian/Ubuntu)

```sh
sudo apt install \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
    librsvg2-dev libssl-dev libsoup-3.0-dev pkg-config \
    build-essential curl

# Install Rust + Node toolchains via rustup / nvm.
pnpm install
pnpm tauri dev   # development run
pnpm tauri build # produce AppImage + .deb
```

#### macOS

```sh
xcode-select --install   # if you don't already have CLT

# Install Rust + Node via your package manager of choice.
pnpm install
pnpm tauri dev
pnpm tauri build         # produces .dmg
```

#### Windows

```powershell
# Install Visual Studio 2022 or later with the "Desktop development
# with C++" workload. WebView2 ships with Windows 11 by default; on
# Windows 10 you may need to install Microsoft's WebView2 Runtime.

# Install Rust + Node + pnpm via your package manager.
pnpm install
pnpm tauri dev
pnpm tauri build         # produces NSIS .exe installer
```

## What gets installed where

Rekindle stores user data under each platform's standard app-data
directory:

| OS | Path |
|----|------|
| Windows | `%APPDATA%\com.rekindle.app\` |
| macOS | `~/Library/Application Support/com.rekindle.app/` |
| Linux | `~/.local/share/com.rekindle.app/` |

Within that directory you'll find:

- `db.sqlite3` — local message and friend cache.
- `stronghold/<identity_pubkey>.stronghold` — your encrypted vault.
  Each identity has its own file.
- `file_cache/<community_id>/` — Lost Cargo file chunks per
  community.
- `veilid/` — Veilid node storage.

If you want to **wipe and start over** (forget all identities, all
communities, all messages), close the app and delete the entire
`com.rekindle.app/` directory. There is no central account to also
delete — your identity exists only in your local vault.

## URL-scheme registration

The installer registers the `rekindle://` URL scheme so that invite
links (`rekindle://invite/...`) launch Rekindle when clicked from a
browser. On Linux this requires a `.desktop` file in
`~/.local/share/applications/`; the `.deb` and AppImage versions
both install one.

If invite links aren't being captured by Rekindle, check your OS's
default-application settings for the `rekindle` URL scheme.

## Autostart

Rekindle includes the Tauri `autostart` plugin. To launch Rekindle on
system login, open **Settings → General → Launch on login**. The
autostart entry is registered with the OS's standard mechanism
(Login Items on macOS, registry `Run` key on Windows, `.desktop` in
`autostart/` on Linux).

## Uninstall

| OS | How |
|----|-----|
| Windows | Settings → Apps → Rekindle → Uninstall |
| macOS | Drag `Rekindle.app` to Trash; optionally delete `~/Library/Application Support/com.rekindle.app/` |
| Linux (`.deb`) | `sudo apt remove rekindle` |
| Linux (AppImage) | Delete the `.AppImage` file; optionally delete `~/.local/share/com.rekindle.app/` |

Uninstalling does **not** delete your identity vault by default — if
you reinstall later, your friends, communities, and history come
back. To fully wipe, also delete the app-data directory listed
above.

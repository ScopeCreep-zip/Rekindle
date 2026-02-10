# Xf1re Installer - Extracted File Manifest

**Source:** `xf1re_installer.exe` (11,003,604 bytes, NSIS v3.10 Unicode)
**Extracted:** 262 files, 30,859,242 bytes uncompressed
**Date:** 2026-02-10

## Identification

- **Build:** Release155b "TitanStrike" (original Xfire codebase)
- **PDB:** `p:\buildvm_share\ua\branches\Release155b_titanstrike\src.cpp\scoville\client\UnicodeRelease\Xfire.pdb`
- **Version:** 1.0.0.13133 (FileVersion 13133)
- **Copyright:** Xfire Inc. 2004
- **Framework:** Native Win32 C++ (MSVCR71 / Visual C++ 2003 runtime)
- **Xf1re modification:** Server addresses patched from `*.xfire.com` → `*.xf1re.com`

## Main Executables

| File | Size | Type | Purpose |
|------|------|------|---------|
| `Xfire.exe` | 3,560,832 | PE32 (GUI) Intel 80386 | **Main client binary** - native Win32 C++ |
| `xfire64.exe` | 258,944 | PE32+ (GUI) x86-64 | 64-bit companion (overlay/game detection) |
| `xfencoder.exe` | 954,368 | PE32 (GUI) Intel 80386 | Video encoder (uses avcodec) |
| `updater.exe` | 37,888 | PE32 (GUI) Mono/.NET | Auto-updater (only .NET binary) |

## DLLs

| File | Size | Type | Purpose |
|------|------|------|---------|
| `icons.dll` | 11,794,816 | PE32 (DLL) | **3,845 game icons** in ICONS resource section |
| `TitanStrikeSupportDll.dll` | 203,136 | PE32 (DLL) | Game overlay support (TitanStrike) |
| `xfire_toucan_46139.dll` | 1,030,528 | PE32 (DLL) | Toucan P2P/file transfer subsystem |
| `xfcodec.dll` | 42,880 | PE32 (DLL) | Voice chat codec (32-bit) |
| `xfcodec64.dll` | 28,544 | PE32+ (DLL) | Voice chat codec (64-bit) |
| `avcodec-52.dll` | 503,168 | PE32 (DLL) | FFmpeg audio/video codec |
| `avutil-50.dll` | 61,824 | PE32 (DLL) | FFmpeg utilities |

## Language DLLs (16 total)

All PE32 resource DLLs containing localized strings:
`xfire_lang_{1337,da,de,es,fr,hu,it,ja,ko,nl,no,pl,pt,ru,sv,us,zh,zh_tw}.dll`

## Configuration

| File | Size | Purpose |
|------|------|---------|
| `xfire_games.ini` | 2,005,467 | **Game detection database** - 2MB INI file with registry keys, process names, launch commands for hundreds of games |
| `Quicktime.qtp` | 12,953 | QuickTime player template |
| `icon.ico` | 3,774 | Application icon |
| `license.txt` | 27,471 | License agreement |

## Skins

| File | Size | Contents |
|------|------|----------|
| `skins/Symbiosis.zip` | 619,882 | **Default skin: 529 files, 33 dirs** |

Symbiosis skin structure:
- `Skin.xml` — master skin definition, includes all components
- `Themes.xml` — complete color palette (78 named RGBA colors)
- `Strings.xml` — UI text strings
- `MainWindow.xml` — buddy list layout (tile-based positioning)
- `ChatWindow.xml` — chat window layout
- `GroupChatWindow.xml` — group chat layout
- `Popups.xml` — popup/notification layouts
- `Components/` — reusable UI component definitions
- `Images/` — 400+ GIF assets (frames, buttons, icons, scrollbars, etc.)
- `XIG/` — in-game overlay skin (v1)
- `XIG2/` — in-game overlay skin (v2, more modular)
- `XfireSkin.dtd` — DTD schema for skin XML

## Sounds

| File | Size |
|------|------|
| `sounds/defaults.zip` | 114,559 |
| `sounds/classic.zip` | 72,034 |

## Templates (HTML/JS/CSS)

Info view templates rendered in embedded browser:
- `templates/` — top-level HTML templates (.tmpl files)
- `templates/infoview/` — rich info panels per feature
- `templates/infoview/scripts/` — 19 JS files (user info, activity reports, clans, etc.)
- `templates/infoview/styles/` — 6 CSS files
- Game-specific info views: `bf2/`, `cs/`, `css/`, `wow/`, `aao/`, etc.

Key templates:
- `friends.tmpl` — friend list info view
- `user.tmpl` — user profile info view
- `login.tmpl` — login form
- `registration.tmpl` — account registration
- `about.tmpl` — about dialog

## NSIS Installer Files

| Dir | Contents |
|-----|----------|
| `$PLUGINSDIR/` | NSIS plugins: ShellLink, System, UserInfo, nsDialogs, nsExec, nsProcess |
| `$PLUGINSDIR/modern-header.bmp` | Installer header image |
| `$PLUGINSDIR/modern-wizard.bmp` | Installer wizard image |
| `$WINDIR/System32/xfcodec.dll` | Voice codec installed to System32 |
| `$WINDIR/SysWOW64/xfcodec.dll` | Voice codec installed to SysWOW64 |
| `$APPDATA/Xfire/` | AppData mirror (games ini, skins, sounds, templates) |

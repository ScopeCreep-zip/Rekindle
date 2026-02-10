# Reverse Engineering & Protocol Resources

## Protocol Documentation

- **OpenFire Protocol Spec** (definitive reference)
  - Repo: https://github.com/iainmcgin/openfire
  - Doc: `OpenFire/docs/xfire_protocol/index.xhtml`
  - Author: Iain McGinniss (2007), built on xfirelib team's RE work

- **IMFreedom Xfire Knowledge Base**
  - https://kb.imfreedom.org/protocols/xfire/
  - Detailed packet reference with authentication flow

## Open-Source Xfire Implementations

### Protocol Libraries
| Project | Language | Repo | Notes |
|---------|----------|------|-------|
| xfirelib | C++ | https://sourceforge.net/projects/libxfire/ | Original RE'd library |
| OpenFire | Java | https://github.com/iainmcgin/openfire | LGPL, best protocol docs |
| XfireKit | C++ | https://github.com/jabwd/XfireKit | "Most advanced" open-source lib |
| MacFire | Obj-C | http://macfire.org/ | Mac OS X implementation |

### Client Plugins (full implementations)
| Project | Platform | Repo | Notes |
|---------|----------|------|-------|
| gfire | Pidgin/C | https://github.com/gfireproject/gfire | **Most complete** - GPLv3 |
| Xblaze | Adium/iOS | https://github.com/jasarien/Xblaze-iOS | First Mac client |
| BlackFire | Mac/Obj-C | https://github.com/jabwd/BlackFire | Unofficial Mac client |

### Server Emulators
| Project | Language | Repo | Notes |
|---------|----------|------|-------|
| PFire | C#/.NET | https://github.com/darcymiranda/PFire | **Active**, MIT, targets v1.127 |

### Key Reference Files in gfire
```
src/gf_protocol.c/h        # Core protocol
src/gf_p2p.c/h              # P2P communication
src/gf_p2p_natcheck.c/h     # NAT traversal
src/gf_game_detection.c/h   # Game detection (+ _linux.c, _win.c)
src/gf_server_query_*.c     # Server query protocols
src/gf_chat.c/h             # Group chat
```

### Key Reference Files in PFire
```
src/PFire.Core/Protocol/Messages/Inbound/    # Client->server handlers
src/PFire.Core/Protocol/Messages/Outbound/   # Server->client builders
src/PFire.Core/Protocol/XFireAttributes.cs   # Attribute type system
```

## Revival Projects

- **Xf1re** (xf1re.com) - Fan-driven revival, beta July 2024, 5000+ users
  - The installer in this repo is from this project

## Tauri 2 Resources

| Resource | URL |
|----------|-----|
| Tauri 2 Docs | https://v2.tauri.app/ |
| Tauri IPC Guide | https://v2.tauri.app/concept/inter-process-communication/ |
| Window Customization | https://v2.tauri.app/learn/window-customization/ |
| System Tray | https://v2.tauri.app/learn/system-tray/ |
| Plugin Directory | https://v2.tauri.app/plugin/ |
| tauri-plugin-decorum | https://github.com/clearlysid/tauri-plugin-decorum |
| tauri-controls | https://github.com/agmmnn/tauri-controls |
| HuLa (Tauri IM reference) | https://github.com/HuLaSpark/HuLa |

## Konductor (Dev Environment)

| Resource | URL |
|----------|-----|
| Konductor Repo | https://github.com/braincraftio/konductor |
| Frontend Devshell | `nix develop .#frontend` (includes Tauri 2 deps) |
| Versions File | `src/lib/versions.nix` (SSOT for all package versions) |

Konductor provides the `frontend` devshell with:
- Rust 1.92+, Node.js 22, pnpm
- GTK, WebKitGTK, OpenSSL (Tauri 2 Linux deps)
- Playwright + bundled browsers (E2E testing)
- 13 linters + 8 formatters with hermetic configs

## Rust Crate References

| Crate | Purpose | Docs |
|-------|---------|------|
| tauri | App framework | https://docs.rs/tauri/2 |
| nom | Binary protocol parsing | https://docs.rs/nom |
| binrw | Declarative binary read/write | https://docs.rs/binrw |
| tokio | Async runtime | https://docs.rs/tokio |
| tokio-util | Codec/Framed for packet I/O | https://docs.rs/tokio-util |
| sha1 | SHA-1 hashing | https://docs.rs/sha1 |
| bytes | Zero-copy byte buffers | https://docs.rs/bytes |
| sysinfo | Cross-platform process enum | https://docs.rs/sysinfo |
| tracing | Structured logging | https://docs.rs/tracing |
| serde | Serialization | https://docs.rs/serde |

## Xfire Timeline

| Year | Event |
|------|-------|
| 2002 | Founded as Ultimate Arena, Inc. |
| 2004 | Public beta, 1M users by August |
| 2006 | Acquired by Viacom for $102M |
| 2010 | Sold to Titan Gaming |
| 2012 | Peak: 22M+ registered users, 3000+ games |
| 2015 | Shutdown (June 12 web, June 27 client) |
| 2016 | Last services shut down (April 30) |
| 2024 | Xf1re revival beta launches (July) |

## Historical / Archival

- ArchiveTeam Xfire: https://wiki.archiveteam.org/index.php/Xfire
- Wikipedia: https://en.wikipedia.org/wiki/Xfire

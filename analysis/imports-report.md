# PE Import Analysis Report

## Xfire.exe

**Type:** PE32 (GUI) Intel 80386, native Win32 C++
**Runtime:** MSVCR71.DLL (Visual C++ 2003)
**Linker:** Microsoft Visual C++ 7.1
**Build:** Release155b TitanStrike
**Version:** 13133

### Sections

| Section | Virtual Address | Raw Size | Virtual Size | Entropy | Notes |
|---------|----------------|----------|--------------|---------|-------|
| `.text` | 0x00001000 | 2,521,600 | 2,521,424 | 6.57 | Code section (~2.4MB) |
| `.rdata` | 0x00269000 | 360,960 | 360,542 | 5.68 | Read-only data (strings, vtables) |
| `.data` | 0x002c2000 | 19,456 | 68,400 | 5.12 | Initialized data |
| `.CRT` | 0x002d3000 | 1,024 | 992 | 4.66 | C runtime init |
| `.rsrc` | 0x002d4000 | 650,240 | 650,056 | 7.06 | Resources (icons, skin, embedded GIFs) |

### Import Summary (15 DLLs, ~357 functions)

| DLL | Count | Category |
|-----|-------|----------|
| `USER32.dll` | 104 | Window management, input, messages |
| `KERNEL32.dll` | 85 | Process, memory, file, thread, sync |
| `GDI32.dll` | 45 | Graphics drawing, fonts, bitmaps |
| `WS2_32.dll` | 28 | Winsock2 networking (TCP + UDP) |
| `WININET.dll` | 26 | HTTP client (web requests) |
| `WINMM.dll` | 12 | Audio (mixer, wave input, sound playback) |
| `ole32.dll` | 12 | COM/OLE (browser embedding) |
| `ADVAPI32.dll` | 11 | Registry, security tokens |
| `AVIFIL32.dll` | 9 | AVI video file handling |
| `COMCTL32.dll` | 5 | Common controls (ImageList, TrackMouse) |
| `OLEAUT32.dll` | 5 | OLE Automation (BSTR, Variant) |
| `VERSION.dll` | 5 | File version info |
| `SHELL32.dll` | 4 | Shell integration (AppBar, DragDrop) |
| `MSIMG32.dll` | 2 | Alpha blending, transparency |
| `IMM32.dll` | 2 | Input method (CJK support) |
| `COMDLG32.dll` | 1 | Common dialogs |

### Key Networking Imports (WS2_32.dll)
```
connect, send, recv, sendto, recvfrom    — TCP and UDP I/O
socket, bind, closesocket                — Socket lifecycle
setsockopt, getsockopt, getsockname      — Socket configuration
ioctlsocket                              — Non-blocking mode
WSAStartup, WSACleanup                   — Winsock init/cleanup
WSAAsyncSelect, WSAAsyncGetHostByName    — Async networking
WSAGetLastError, WSASetLastError         — Error handling
htons, htonl, ntohs, ntohl              — Byte order conversion
select                                   — I/O multiplexing
```

### Notable: No Direct Crypto Imports
SHA1 is **statically linked** — no DLL import for crypto functions. The `UltimateArena` salt and double-SHA1 auth hash are computed by code compiled directly into the .text section.

### Resources Embedded in Xfire.exe

| Type | Count | Description |
|------|-------|-------------|
| FLAGS | 239 | Country flag images for user profiles |
| RT_ICON | 32 | Application icons (multiple sizes) |
| RT_GROUP_ICON | 23 | Icon groups |
| RT_BITMAP | 10 | Bitmap resources |
| GIFS | 6 | Animated GIF resources |
| PNGS | 2 | PNG resources |
| ADS | 2 | Advertisement placeholders |
| SKIN | 1 | Embedded default skin data |
| SOUND | 1 | Embedded sound |
| RT_CURSOR | 1 | Custom cursor |
| RT_VERSION | 1 | Version info |
| RT_MANIFEST | 1 | Application manifest |

---

## icons.dll

**Type:** PE32 (DLL) Intel 80386
**Size:** 11,794,816 bytes (11.2 MB)
**Purpose:** Game icon resource library

### Sections
| Section | Raw Size | Notes |
|---------|----------|-------|
| `.text` | 28,672 | Minimal code |
| `.rdata` | 8,192 | Read-only data |
| `.data` | 4,096 | Data |
| `.rsrc` | **11,722,752** | 99.4% of the DLL is resources |
| `.reloc` | 20,480 | Relocations |

### Resources
- **ICONS: 3,845 entries** — one icon per supported game
- No exports — purely a resource container loaded at runtime

---

## updater.exe

**Type:** PE32 (GUI) Mono/.NET Assembly
**Size:** 37,888 bytes
**Import:** `mscoree.dll → _CorExeMain` (CLR bootstrap)
**Purpose:** Auto-updater for the Xf1re client

The only .NET binary in the package. Minimal — just bootstraps the CLR and runs the updater logic. Could be decompiled with ILSpy for full source recovery.

---

## Key Architectural Insights for Rekindle

1. **Xfire.exe is a single monolithic native Win32 binary** — all protocol, UI, game detection, P2P, voice chat, and overlay logic is compiled into one 3.5MB executable.

2. **No framework dependencies** — uses raw Win32 API (USER32, GDI32, KERNEL32) with MSVCR71 runtime. No MFC, no ATL, no Qt.

3. **Custom XML skinning engine** — tile-based layout with z-ordering, relative positioning, and theme colors. Skins are ZIP-packaged with XML + GIF images.

4. **Embedded browser** — uses OLE/COM (ole32, OLEAUT32) to embed MSHTML/Internet Explorer for info views, rendered from `.tmpl` templates with JS/CSS.

5. **Multiple network layers:**
   - TCP to game server (port 25999) — main protocol
   - UDP for P2P (file transfer, voice chat)
   - HTTP via WININET — web services, 3rd party auth
   - UPnP for NAT traversal

6. **icons.dll is a pure resource DLL** — 3,845 game icons. We can extract all of these for our icon library.

7. **239 country flags** embedded in Xfire.exe resources — for user profiles.

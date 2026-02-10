# Reverse Engineering Workflow

Step-by-step guide for analyzing the Xfire/Xf1re installer and client binary.

## Phase 1: Unpack the NSIS Installer

The `xf1re_installer.exe` is a Nullsoft (NSIS) self-extracting archive. Extract without executing:

```bash
# Install 7z if needed
brew install p7zip   # macOS
# sudo apt install p7zip-full  # Linux

# Extract installer contents
7z x xf1re_installer.exe -o./unpacked

# Inspect what was extracted
file ./unpacked/*
ls -la ./unpacked/
```

**What to look for:**
- Main `.exe` client binary (likely `xf1re.exe` or similar)
- DLLs (overlay hooks, LSP providers, protocol libs)
- Configuration files (game lists, server addresses, skins)
- NSIS scripts (`[NSIS].nsi`) showing install logic, registry keys, file placements
- Data files (game detection databases, certificate bundles)

## Phase 2: Triage Extracted Files

```bash
# Identify all PE executables and DLLs
file ./unpacked/* | grep -i "PE32\|DLL\|executable"

# Look for interesting strings in each binary
for f in ./unpacked/*.exe ./unpacked/*.dll; do
  echo "=== $f ==="
  strings "$f" | grep -iE "xfire|UA01|UltimateArena|cs\.xfire|25999|SHA|login|chat|overlay|hook|inject|game" | head -30
done

# Check for .NET assemblies (Xf1re might use modern tech)
file ./unpacked/*.exe | grep -i "\.NET\|CLR\|Mono"

# Check for packed/obfuscated binaries
for f in ./unpacked/*.exe; do
  echo "=== $f ==="
  strings "$f" | grep -iE "UPX|VMProtect|Themida|ASPack|PECompact" | head -5
done
```

## Phase 3: Static Analysis with Ghidra

```bash
# Headless analysis
analyzeHeadless /tmp/ghidra_project Rekindle \
  -import ./unpacked/<main_binary> \
  -postScript ExportFunctions.java \
  -scriptPath /path/to/ghidra/scripts

# Or launch Ghidra GUI
ghidraRun
# File -> Import -> select binary -> Auto-analyze -> Yes to all
```

**Analysis targets in Ghidra:**
1. Search strings for `"UA01"`, `"UltimateArena"`, `cs.xfire.com`
2. Find `connect()` / `WSAConnect()` calls - locate the network initialization
3. Find `send()` / `recv()` calls - trace packet construction and parsing
4. Look for SHA-1 implementation (or imports from crypto libs)
5. Find `CreateToolhelp32Snapshot` / `Process32First` - game detection via process enum
6. Find `WSCInstallProvider` / `WSPStartup` - LSP-based game server detection
7. Find DirectX/OpenGL hooks (`Direct3DCreate9`, `wglSwapBuffers`) - overlay system

## Phase 4: Static Analysis with radare2/rizin

```bash
# Open in analysis mode
r2 -A ./unpacked/<main_binary>

# Find strings
iz~UA01
iz~UltimateArena
iz~xfire

# List imports
ii

# Find network functions
ii~connect
ii~send
ii~recv
ii~WSA

# Find crypto functions
ii~SHA
ii~Crypt
ii~hash

# Analyze main function
s main
pdf

# Cross-references to interesting strings
axt @@ str.*UA01*
```

## Phase 5: Dynamic Analysis (Windows VM only)

> **WARNING**: Only run in an isolated VM with snapshot capability.

```bash
# Network capture with Wireshark
# Filter: tcp.port == 25999

# API monitoring with API Monitor or x64dbg
# Set breakpoints on: connect, send, recv, CreateToolhelp32Snapshot

# Use Procmon to observe:
# - File system access (config files, game lists)
# - Registry access (install paths, settings)
# - Network connections
# - Process creation
```

## Phase 6: Protocol Verification

Compare findings against the known protocol spec (`.claude/docs/xfire-protocol.md`):

1. **Verify packet structure** - Do the binary's packet builders match the documented format?
2. **Check for new packet types** - Xf1re may have added packets not in the original protocol
3. **Verify auth scheme** - Is it still double-SHA1 with "UltimateArena"?
4. **Check for TLS/encryption** - Xf1re may have added transport encryption
5. **Look for new features** - Any protocol extensions for modern functionality?

## Tools Summary

| Tool        | Purpose                          | Install                        |
|-------------|----------------------------------|--------------------------------|
| 7z          | Unpack NSIS installer            | `brew install p7zip`           |
| Ghidra      | Disassembly & decompilation      | ghidra-sre.org                 |
| radare2     | CLI disassembly & analysis       | `brew install radare2`         |
| rizin       | radare2 fork, improved UX        | `brew install rizin`           |
| strings     | Extract printable strings        | Built-in (binutils)            |
| file        | Identify file types              | Built-in                       |
| Wireshark   | Network packet capture           | wireshark.org                  |
| x64dbg      | Windows debugger                 | x64dbg.com (Windows only)      |
| PE-bear     | PE header viewer                 | github.com/hasherezade/pe-bear |
| Detect It Easy | Packer/compiler detection     | github.com/horsicq/DIE-engine  |
| binwalk     | Firmware/binary analysis         | `brew install binwalk`         |

## File Organization

Keep extracted and analyzed artifacts organized:

```
unpacked/              # Raw NSIS extraction output
analysis/
  strings/             # Extracted strings from each binary
  ghidra/              # Ghidra project files
  pcaps/               # Network captures
  notes/               # Analysis notes per binary
src/                   # Rust rewrite source (workspace root)
  rekindle-protocol/   # Protocol library crate
  rekindle-client/     # Client application crate
  rekindle-overlay/    # Overlay system crate
```

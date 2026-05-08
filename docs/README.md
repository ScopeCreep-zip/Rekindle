# Rekindle Technical Documentation

Rekindle is a decentralised peer-to-peer chat and community platform built
with Tauri 2, SolidJS, and the Veilid network. It provides end-to-end
encrypted 1:1 messaging, DMs and group DMs, communities with
channels/voice/video, cross-device sync, and cross-platform game
detection — all without any central server. The community layer follows
the **Communities v2.0** flat-governance model: no coordinator, no
privileged nodes, every member is a full peer.

## Reading paths

This documentation is organised by audience. Pick the reading path that
matches what you're trying to do.

### I want to **use** Rekindle

Start with [`user/`](user/) — install guides, common tasks, and the FAQ.

### I want to **contribute** to Rekindle

Start with [`../CONTRIBUTING.md`](../CONTRIBUTING.md) at the repo root,
then [`contributor/development.md`](contributor/development.md) for
hands-on dev environment setup, and the rest of [`contributor/`](contributor/)
for testing, style, and release process.

### I want to **understand the system**

Start with [`../ARCHITECTURE.md`](../ARCHITECTURE.md) for a one-page
bird's-eye view, then drill into [`architecture/`](architecture/) and
[`protocol/`](protocol/) by subsystem.

### I want to **audit the cryptography**

Start with [`security/overview.md`](security/overview.md) for the
encryption-layer stack, then [`security/threat-model.md`](security/threat-model.md)
for what we protect against and what we do not, and
[`decisions/`](decisions/) for the architectural decision records that
shaped the cryptographic choices.

## Document index

### Architecture (`architecture/`)

| Document | Description |
|----------|-------------|
| [`overview.md`](architecture/overview.md) | System architecture, layer responsibilities, data-flow diagrams |
| [`communities.md`](architecture/communities.md) | Chiral-network v2.0 — flat SMPL governance, three-path delivery, CRDT merge, plate-gate scaling |
| [`crates.md`](architecture/crates.md) | All 22 workspace crates, tier hierarchy, daemon/CLI track |
| [`data-layer.md`](architecture/data-layer.md) | SQLite schema, Stronghold vault, DHT record layout |
| [`frontend.md`](architecture/frontend.md) | SolidJS frontend — windows, components, stores, handlers, IPC layer |
| [`tauri-backend.md`](architecture/tauri-backend.md) | Tauri shell — commands, channels, services, app state |
| [`voice.md`](architecture/voice.md) | Voice pipeline (cpal threading, Opus, RNNoise, AEC3, jitter buffer, mixer, MCU pattern, mutual-aid SFU) |
| [`game-detect.md`](architecture/game-detect.md) | Cross-platform process scanning, JSON game DB, rich presence |
| [`files.md`](architecture/files.md) | Lost Cargo file delivery (chunking, BLAKE3, swarm fetch, LRU cache) |
| [`sync.md`](architecture/sync.md) | Cross-device sync (personal DFLT record, pairing, gap detection) |
| [`daemon-cli.md`](architecture/daemon-cli.md) | Daemon + CLI architecture (`rekindle-node`, `rekindle-cli`, Noise IK IPC bus) |
| [`ui-skin.md`](architecture/ui-skin.md) | Frameless titlebar, Xfire colour palette, window catalogue, asset usage |

### Protocol (`protocol/`)

| Document | Description |
|----------|-------------|
| [`overview.md`](protocol/overview.md) | Network protocol, Veilid integration, message lifecycle, gossip mesh, DHT record layouts |
| [`relay.md`](protocol/relay.md) | Strand Relay forwarding + Mobile Push Relay (3-tier escalation) |

### Security (`security/`)

| Document | Description |
|----------|-------------|
| [`overview.md`](security/overview.md) | Five-layer encryption stack, identity model, threat analysis |
| [`threat-model.md`](security/threat-model.md) | STRIDE/LINDDUN-structured threat model, vulnerable-user posture |
| [`crypto-primitives.md`](security/crypto-primitives.md) | Per-primitive selection rationale (Ed25519, X25519, AES-256-GCM, XChaCha20-Poly1305, BLAKE3, HKDF-SHA256, Argon2id, Noise IK) |
| [`privacy-properties.md`](security/privacy-properties.md) | What Veilid gives, what Rekindle adds, what is not protected |

### Decisions (`decisions/`) — Architectural Decision Records

MADR 4.0 ADRs documenting why the project made each load-bearing choice.
Append-only — superseded ADRs stay in place with a "Superseded by" link.

| ADR | Title |
|-----|-------|
| [0001](decisions/0001-veilid-as-transport.md) | Adopt Veilid as the sole transport substrate |
| [0002](decisions/0002-signal-protocol-for-1to1.md) | Use the Signal Protocol for 1:1 friend messaging |
| [0003](decisions/0003-flat-smpl-governance.md) | Flat SMPL governance replaces the v1.0 coordinator model |
| [0004](decisions/0004-tauri-2-frontend.md) | Use Tauri 2 + SolidJS as the desktop frontend |
| [0005](decisions/0005-daemon-cli-track.md) | Add a daemon + CLI track alongside the Tauri desktop app |

See [`decisions/README.md`](decisions/README.md) for how to write a new ADR.

### Contributor (`contributor/`)

| Document | Description |
|----------|-------------|
| [`development.md`](contributor/development.md) | Dev environment, build commands, dependency overview |
| [`testing.md`](contributor/testing.md) | Test strategy (unit / mock IPC / E2E / property tests) |
| [`style-guide.md`](contributor/style-guide.md) | Rust + TypeScript + Tailwind conventions |
| [`release-process.md`](contributor/release-process.md) | Tagging, building, distributing, signing |

### User (`user/`)

| Document | Description |
|----------|-------------|
| [`getting-started.md`](user/getting-started.md) | Install, create an identity, send your first message |
| [`install.md`](user/install.md) | Per-platform install instructions and source-build notes |
| [`how-to.md`](user/how-to.md) | Common-task walkthroughs (add friend, voice call, file send, pair device, …) |
| [`faq.md`](user/faq.md) | Frequently asked questions |

### Top-level

| Document | Description |
|----------|-------------|
| [`roadmap.md`](roadmap.md) | Implementation phases and completion status |
| [`glossary.md`](glossary.md) | Project-specific vocabulary (DHT, SMPL, MEK, Plate Gate, VICE, Schwarzschild principle, etc.) |

## Repository structure

```
src/                                Frontend (SolidJS + Tailwind 4)
src-tauri/                          Tauri 2 Rust backend
  src/
    lib.rs                          Application entry, command registry, plugin setup
    state.rs                        AppState (40+ fields)
    db.rs                           SQLite pool, schema versioning
    commands/                       IPC command handlers (~220 commands)
      community/                    Community subcommands (30 modules)
    channels/                       Event type definitions (Rust → Frontend)
    services/                       Background services
      community/                    Community gossip, governance, presence, etc.
      veilid/                       Node lifecycle, dispatch loop
      voice/                        Voice send/receive/MCU loops
      cross_device_sync/            Multi-device sync
      relay/                        Strand Relay forwarding
      dm/                           Direct messages
      search/                       Message search
  migrations/001_init.sql           SQLite schema (single file, edit in place)
crates/                             22 workspace members — see architecture/crates.md
  # Tiered pure-logic crates (zero Tauri, zero veilid-core)
  rekindle-types/                   Tier 1: shared IDs, enums, error taxonomy
  rekindle-secrets/                 Tier 2: keys, MEK, signing — sole crypto boundary
  rekindle-codec/                   Tier 3: signed envelope build/verify, dedup
  rekindle-records/                 Tier 3: DHT record lifecycle, retry, SMPL schemas
  rekindle-utils/                   Time helpers, shared utilities
  rekindle-route/                   Tier 4: private route lifecycle, peer cache
  rekindle-gossip/                  Tier 5: gossip mesh primitives
  rekindle-governance/              Tier 6: pure CRDT merge, permission resolution
  rekindle-dm/                      Tier 7: DM/group DM (SMPL records, X25519-derived MEK)
  rekindle-calls/                   Tier 7: direct call key derivation
  rekindle-files/                   Tier 7: chunked P2P file transfer (Lost Cargo)
  rekindle-link-preview/            Tier 7: OpenGraph fetcher (sandboxed)
  rekindle-video/                   Tier 7: video / screen-share fragmentation

  # Cross-cutting integration crates
  rekindle-sync/                    Cross-device sync (fetch, gap, history, watching)
  rekindle-protocol/                Veilid + Cap'n Proto + DHT — used by the desktop app
  rekindle-crypto/                  Identity, Signal Protocol session manager, dht keys
  rekindle-game-detect/             Cross-platform game detection
  rekindle-voice/                   Opus codec, audio I/O, jitter, mixer, transport
  rekindle-e2e-server/              HTTP IPC bridge for E2E tests

  # Daemon/CLI track (alternate frontend, parallel to the Tauri desktop app)
  rekindle-transport/               Unified Veilid boundary (broadcast/subscriptions split)
  rekindle-node/                    Daemon: owns Veilid, serves a Noise-IK encrypted IPC bus
  rekindle-cli/                     CLI/TUI client of rekindle-node over the IPC bus
schemas/                            Cap'n Proto schema definitions (8 files)
e2e/                                Playwright E2E tests
```

## Two frontends, one protocol

Rekindle ships two ways to talk to the network:

1. **Desktop app** (`src-tauri/`) — Tauri 2 process that links
   `rekindle-protocol` directly and runs the Veilid node in-process. This
   is the primary user-facing build today.
2. **Daemon + CLI** (`rekindle-node` + `rekindle-cli`) — a long-running
   daemon that owns the Veilid node and exposes it over an encrypted IPC
   bus, with a `clap`/`ratatui` CLI/TUI as the first client.
   `rekindle-transport` is the sole Veilid boundary on this track. Future
   clients (other frontends, automation, bridges) plug into the same bus.

Both frontends speak the same Veilid protocol, the same SMPL governance,
and the same Signal/MEK encryption — they differ only in how the local
process is structured.

# Rekindle Technical Documentation

Rekindle is a decentralized peer-to-peer chat and community platform built with
Tauri 2, SolidJS, and the Veilid network. It provides end-to-end encrypted 1:1
messaging, DMs and group DMs, communities with channels/voice/video, cross-device
sync, and cross-platform game detection — all without any central server.
Rekindle draws visual inspiration from the classic Xfire gaming client.

The community layer follows the **Communities v2.0** flat-governance model:
no coordinator process, no privileged nodes, every member is a full peer.
Distributed state is held in SMPL multi-writer DHT records with `o_cnt:0`
(creation keypair discarded after genesis). All peer writes are merged
client-side with a deterministic CRDT.

## Document Index

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | System architecture, layer responsibilities, data flow diagrams |
| [protocol.md](protocol.md) | Network protocol, Veilid integration, message lifecycle, gossip mesh |
| [security.md](security.md) | Encryption layers, identity model, threat analysis |
| [data-layer.md](data-layer.md) | SQLite schema, Stronghold vault, DHT record layout |
| [frontend.md](frontend.md) | SolidJS frontend, routing, stores, IPC layer |
| [crates.md](crates.md) | Pure Rust crate reference (16 workspace crates) |
| [tauri-backend.md](tauri-backend.md) | Tauri application shell, commands, events, services |
| [development.md](development.md) | Development environment, build commands, testing, conventions |
| [roadmap.md](roadmap.md) | Implementation phases and completion status |

## Repository Structure

```
src/                                Frontend (SolidJS + Tailwind 4)
src-tauri/                          Tauri 2 Rust backend
  src/
    lib.rs                          Application entry, command registry, plugin setup
    state.rs                        AppState (40+ fields)
    db.rs                           SQLite pool, schema versioning
    commands/                       IPC command handlers (~170 commands)
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
crates/
  rekindle-types/                   Tier 1: shared IDs, enums, error taxonomy
  rekindle-secrets/                 Tier 2: keys, MEK, signing — sole crypto boundary
  rekindle-codec/                   Tier 3: signed envelope build/verify, dedup
  rekindle-records/                 Tier 3: DHT record lifecycle, retry, SMPL schemas
  rekindle-utils/                   Time helpers, shared utilities
  rekindle-route/                   Tier 4: private route lifecycle, peer cache
  rekindle-gossip/                  Tier 5: gossip mesh primitives (D-fanout, dedup, Lamport)
  rekindle-governance/              Tier 6: pure CRDT merge, permission resolution
  rekindle-dm/                      Tier 7: DM/group DM (SMPL records, X25519-derived MEK)
  rekindle-files/                   Tier 7: chunked P2P file transfer (Lost Cargo)
  rekindle-link-preview/            Tier 7: OpenGraph fetcher (sandboxed)
  rekindle-video/                   Tier 7: video / screen-share fragmentation
  rekindle-sync/                    Cross-device sync (fetch, gap, history, watching)
  rekindle-protocol/                Veilid networking, Cap'n Proto codec, DHT manager
  rekindle-crypto/                  Identity, Signal Protocol session manager, dht keys
  rekindle-game-detect/             Cross-platform game detection
  rekindle-voice/                   Opus codec, audio I/O, jitter, mixer, transport
  rekindle-e2e-server/              HTTP IPC bridge for E2E tests
schemas/                            Cap'n Proto schema definitions (8 files)
e2e/                                Playwright E2E tests
```

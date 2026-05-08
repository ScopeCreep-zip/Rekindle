# Architecture

A bird's-eye-view of the Rekindle codebase, written for someone new to the
repo. This file's job is to point you at the right place to look for any
given concern. Detailed component docs live under
[`docs/`](docs/README.md).

## What Rekindle is

Rekindle is a 1:1 reimplementation of the classic Xfire gaming chat client,
built on **Tauri 2** (Rust backend + webview frontend), **SolidJS**
(TypeScript), **Tailwind 4**, and the **Veilid** peer-to-peer network. It
provides end-to-end encrypted 1:1 messaging, DMs and group DMs, communities
with channels/voice/video, cross-device sync, and cross-platform game
detection — without any central server.

The community layer follows the **Communities v2.0** flat-governance model:
no coordinator process, no privileged nodes, every member is a full peer.
See [`docs/architecture/communities.md`](docs/architecture/communities.md)
for the chiral-network architecture.

## Layer stack

```
┌─────────────────────────────────────────────────────────┐
│                     SolidJS Frontend                    │
│  Windows, components, stores, handlers, styles          │
│  (src/)                                                 │
├─────────────────────────────────────────────────────────┤
│                   Tauri 2 IPC Bridge                    │
│  ~220 commands, 6 event channels, 7 windows             │
│  Plugin setup, system tray, window lifecycle            │
│  (src-tauri/)                                           │
├─────────────────────────────────────────────────────────┤
│                Pure Rust Crates (Tiers 1–7)             │
│  Tier 1   types          Tier 2  secrets                │
│  Tier 3   codec/records  Tier 4  route                  │
│  Tier 5   gossip         Tier 6  governance             │
│  Tier 7   dm/calls/files/video/link-preview             │
│  (crates/)                                              │
├─────────────────────────────────────────────────────────┤
│                    Veilid Network                       │
│  DHT storage (DFLT + SMPL), app_message routing         │
│  Private + safety routes, XChaCha20-Poly1305 transport  │
└─────────────────────────────────────────────────────────┘
```

A parallel **daemon + CLI** track (`rekindle-node` + `rekindle-cli` +
`rekindle-transport`) runs the same protocol stack with a Noise-IK
encrypted IPC bus instead of an in-process Tauri shell. Both frontends
speak the same Veilid protocol and SMPL governance.

## Top-level layout

| Path | What lives here |
|------|-----------------|
| `src/` | SolidJS frontend (windows, components, stores, handlers, styles) |
| `src-tauri/` | Tauri 2 backend — commands, services, channels, app state, SQLite, Stronghold |
| `crates/` | 22 Rust crates implementing the protocol, crypto, voice, game detection, daemon/CLI |
| `schemas/` | Cap'n Proto schema definitions |
| `e2e/` | Playwright E2E tests |
| `docs/` | All technical documentation (start at [`docs/README.md`](docs/README.md)) |
| `legacy/` | Reverse-engineering artifacts from the original Xfire installer (reference only — do not execute) |
| `.claude/` | Internal design memos, plans, and Claude-specific automation (gitignored) |
| `.github/` | Issue/PR templates, CODEOWNERS |

## Where to look for what

| Concern | Start here |
|---------|------------|
| What does the app do at runtime? | [`docs/architecture/overview.md`](docs/architecture/overview.md) — full layer stack and data flows |
| How are communities structured? | [`docs/architecture/communities.md`](docs/architecture/communities.md) — chiral-network v2.0 |
| What's the wire format? | [`docs/protocol/overview.md`](docs/protocol/overview.md) and [`schemas/`](schemas/) |
| How is the encryption layered? | [`docs/security/overview.md`](docs/security/overview.md) and the rest of [`docs/security/`](docs/security/) |
| Where does data persist? | [`docs/architecture/data-layer.md`](docs/architecture/data-layer.md) — SQLite, Stronghold, DHT |
| How are crates organised? | [`docs/architecture/crates.md`](docs/architecture/crates.md) — every crate, its tier, its role |
| How does the SolidJS UI work? | [`docs/architecture/frontend.md`](docs/architecture/frontend.md) — windows, stores, IPC layer |
| How do Tauri commands and services hook up? | [`docs/architecture/tauri-backend.md`](docs/architecture/tauri-backend.md) |
| How do I get the dev environment running? | [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`docs/contributor/development.md`](docs/contributor/development.md) |
| What's left to build? | [`docs/roadmap.md`](docs/roadmap.md) |

## Crate tier hierarchy

Lower-numbered tiers know nothing about higher tiers. The lowest tiers
(`rekindle-types`, `rekindle-secrets`) contain zero I/O, zero async, zero
side effects.

| Tier | Crates | Role |
|------|--------|------|
| 1 | `rekindle-types` | Shared IDs, enums, error taxonomy |
| 2 | `rekindle-secrets` | All key material, Zeroize, sole crypto boundary |
| 3 | `rekindle-codec`, `rekindle-records` | Signed envelope build/verify, DHT record lifecycle |
| 4 | `rekindle-route` | Private route allocation, peer route cache |
| 5 | `rekindle-gossip` | Gossip mesh primitives (D-fanout, dedup, Lamport) |
| 6 | `rekindle-governance` | Pure CRDT merge, reader-validates permissions |
| 7 | `rekindle-dm`, `rekindle-calls`, `rekindle-files`, `rekindle-video`, `rekindle-link-preview` | Self-contained features |
| Cross-cutting | `rekindle-protocol`, `rekindle-crypto`, `rekindle-voice`, `rekindle-game-detect`, `rekindle-sync`, `rekindle-utils`, `rekindle-e2e-server` | Veilid plumbing, Signal sessions, audio pipeline, scanner, sync workers |
| Daemon/CLI track | `rekindle-transport`, `rekindle-node`, `rekindle-cli` | Sole Veilid boundary + daemon + CLI/TUI |

## Protocol at a glance

- **Transport:** Veilid P2P (no central server). NAT traversal via VICE.
- **Serialisation:** Cap'n Proto for wire (`schemas/`); JSON for IPC.
- **Encryption:**
  - Layer 1 — Veilid transport (XChaCha20-Poly1305, hop-by-hop).
  - Layer 2 — Privacy routing (safety routes for sender, private routes
    for receiver).
  - Layer 3 — Ed25519 signatures on every envelope.
  - Layer 4 — AES-256-GCM with per-channel MEK (communities) or Signal
    Protocol (1:1 friends).
  - Layer 5 — Stronghold vault (Argon2id + XChaCha20-Poly1305) at rest.
- **Identity:** Ed25519 keypairs. No usernames or passwords. Pseudonyms
  per community for unlinkability.
- **Community governance:** SMPL DHT records with `o_cnt: 0` (creation
  keypair shared as infrastructure, not authority). CRDT merge of all
  member subkeys produces consistent state. Reader-validates enforcement.
- **Three-path delivery:** SMPL write (durability) + gossip (latency) +
  watch/inspect (consistency).

## Repository conventions

- **Zero warnings.** Workspace lints are `deny(warnings)`,
  `deny(dead_code)`, `clippy::all = deny`.
- **No `#[allow(dead_code)]`.** Wire it up or delete it.
- **No legacy compatibility shims.** Pre-release; replace fields, drop
  columns, delete old code paths.
- **Database schema** is one file (`src-tauri/migrations/001_init.sql`).
  Edit in place, bump `SCHEMA_VERSION` in `db.rs`.
- **Tailwind** lives in `src/styles/`. No inline classes in components.
- **Frontend is thin.** All business logic in the Rust backend.

For the full set of conventions, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Rekindle is released under the [MIT License](LICENSE).

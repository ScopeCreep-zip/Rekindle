# Rekindle Technical Documentation

Rekindle is a decentralized peer-to-peer chat application built with Tauri 2,
SolidJS, and the Veilid network. It provides end-to-end encrypted messaging,
community channels, voice chat, and cross-platform game detection without any
central server. Rekindle draws visual inspiration from the classic Xfire gaming
client.

## Document Index

| Document | Description |
|----------|-------------|
| [architecture.md](architecture.md) | System architecture, layer responsibilities, data flow diagrams |
| [protocol.md](protocol.md) | Network protocol, Veilid integration, message lifecycle |
| [security.md](security.md) | Encryption layers, identity model, threat analysis |
| [data-layer.md](data-layer.md) | SQLite schema, Stronghold vault, DHT record layout |
| [frontend.md](frontend.md) | SolidJS frontend, routing, stores, IPC layer |
| [crates.md](crates.md) | Pure Rust crate reference (protocol, crypto, game-detect, voice, server) |
| [tauri-backend.md](tauri-backend.md) | Tauri application shell, commands, events, services |
| [development.md](development.md) | Development environment, build commands, testing, conventions |
| [roadmap.md](roadmap.md) | Implementation phases and completion status |

## Repository Structure

```
src/                           Frontend (SolidJS + Tailwind 4)
src-tauri/                     Tauri 2 Rust backend
  src/
    lib.rs                     Application entry point
    commands/                  IPC command handlers
    channels/                  Event type definitions
    services/                  Background services
  migrations/                  SQLite schema
crates/
  rekindle-protocol/           Veilid networking, DHT, Cap'n Proto
  rekindle-crypto/             Ed25519 identity, Signal Protocol, group encryption
  rekindle-game-detect/        Cross-platform game detection
  rekindle-voice/              Opus codec, audio capture/playback, VAD
  rekindle-server/             Community hosting daemon (child process)
schemas/                       Cap'n Proto schema definitions
```

# Rust Crate Reference

Rekindle's business logic is split across four pure Rust crates with zero Tauri
dependency. This separation ensures the protocol, cryptography, game detection,
and voice subsystems can be tested independently and reused outside the Tauri
shell.

## Workspace Structure

```
crates/
├── rekindle-protocol/      Veilid networking, DHT, Cap'n Proto
├── rekindle-crypto/        Ed25519 identity, Signal Protocol, group encryption
├── rekindle-game-detect/   Cross-platform game detection
└── rekindle-voice/         Opus codec, audio I/O, VAD, transport
```

Workspace-level dependencies are defined in the root `Cargo.toml`:
`serde`, `tokio`, `tracing`, `thiserror`, `anyhow`, `parking_lot`, `capnp`, `hex`, `rand`.

Workspace lints enforce `deny(warnings)`, `deny(dead_code)`, and
`clippy::all = deny`.

---

## rekindle-protocol

Veilid networking, DHT record management, message serialization, and routing.

### Module Structure

```
src/
├── lib.rs                  Crate root, Cap'n Proto generated module includes
├── error.rs                Protocol-level error types
├── node.rs                 Veilid node lifecycle (startup, attach, shutdown)
├── routing.rs              Private route allocation and management
├── peer.rs                 Peer address resolution
├── capnp_codec.rs          Cap'n Proto encode/decode helpers
├── messaging/
│   ├── mod.rs              Message type exports
│   ├── envelope.rs         MessageEnvelope construction and parsing
│   ├── sender.rs           Outbound message delivery via app_message
│   └── receiver.rs         Inbound message dispatch
└── dht/
    ├── mod.rs              DHTManager, record operations
    ├── profile.rs          User profile record (8 subkeys)
    ├── presence.rs         Presence data read/write
    ├── friends.rs          Friend list DHT record
    ├── community.rs        Community DHT records (SMPL)
    ├── channel.rs          Channel DHT records
    ├── conversation.rs     Conversation DHT records (per-friend pair)
    ├── account.rs          Account recovery records
    ├── short_array.rs      Short array DHT helper
    └── log.rs              DHT operation logging
```

### Key Types

| Type | Description |
|------|-------------|
| `NodeConfig` | Veilid startup configuration (namespace, storage paths) |
| `DHTManager` | Owns a `RoutingContext`, performs all DHT read/write/watch operations |
| `RoutingManager` | Allocates and maintains private routes for message receiving |
| `MessageEnvelope` | Serialized wrapper for all application messages |
| `MessageSender` | Looks up peer routes and delivers encrypted payloads |
| `MessageReceiver` | Deserializes incoming `app_message` payloads by type |

### External Dependencies

`veilid-core`, `capnp`, `capnpc` (build), `tokio`, `tracing`, `thiserror`, `hex`

---

## rekindle-crypto

Cryptographic operations including Ed25519 identity management, Signal Protocol
session handling, and group media encryption keys.

### Module Structure

```
src/
├── lib.rs                  Crate root, re-exports
├── error.rs                Crypto error types
├── identity.rs             Ed25519 keypair generation and management
├── keychain.rs             Key derivation and conversion (Ed25519 ↔ X25519)
├── dht_crypto.rs           DHT record encryption/signing helpers
├── group/
│   ├── mod.rs              Group encryption exports
│   └── media_key.rs        MEK generation, encrypt/decrypt (distribution not yet wired)
└── signal/
    ├── mod.rs              Signal Protocol session manager
    ├── session.rs          Signal session establishment and message encrypt/decrypt
    ├── prekeys.rs          PreKeyBundle struct (generation/rotation TODO)
    ├── store.rs            Stronghold-backed Signal key storage
    ├── memory_stores.rs    In-memory Signal stores (for testing)
    └── test_stores.rs      Test fixture stores
```

### Key Types

| Type | Description |
|------|-------------|
| `Identity` | Ed25519 keypair with derived X25519 key |
| `SignalSessionManager` | Manages Signal sessions for all peers |
| `PreKeyBundle` | Public keys published to DHT for session establishment |
| `MediaEncryptionKey` | AES-256-GCM symmetric key for community channels |

### Signal Session Flow

```
┌──────────┐                              ┌──────────┐
│  Alice   │                              │   Bob    │
│          │   1. Fetch PreKeyBundle      │          │
│          │ ──────────────────────────→   │ (DHT)   │
│          │                              │          │
│          │   2. X3DH Key Agreement      │          │
│          │   (derive shared secret)     │          │
│          │                              │          │
│          │   3. Initial Message +       │          │
│          │      Identity + Ephemeral    │          │
│          │ ──────────────────────────→   │          │
│          │                              │          │
│          │   4. Bob derives same        │          │
│          │      shared secret           │          │
│          │                              │          │
│          │   5. Double Ratchet active   │          │
│          │ ←────────────────────────→   │          │
└──────────┘                              └──────────┘
```

### External Dependencies

`ed25519-dalek`, `x25519-dalek`, `aes-gcm`, `hkdf`, `sha2`, `rand`,
`zeroize`, `serde`, `thiserror`

---

## rekindle-game-detect

Cross-platform game detection via process scanning and a JSON game database.

### Module Structure

```
src/
├── lib.rs                  Crate root, GameDetector public API
├── error.rs                Detection error types
├── scanner.rs              Process scanning loop (configurable interval)
├── database.rs             JSON game database (process name → game info)
├── rich_presence.rs        Rich presence data (game name, server, elapsed time)
└── platform/
    ├── mod.rs              Platform trait and conditional compilation
    ├── linux.rs            /proc-based process enumeration
    ├── macos.rs            macOS process enumeration
    └── windows.rs          CreateToolhelp32Snapshot-based enumeration
```

### Key Types

| Type | Description |
|------|-------------|
| `GameDetector` | Main entry point — starts scan loop, reports game changes |
| `GameDatabase` | Loaded from JSON, maps process names to game metadata |
| `GameInfo` | Detected game: ID, name, server info, elapsed time |
| `ProcessScanner` | Platform-specific process enumeration trait |

### External Dependencies

`sysinfo`, `serde`, `serde_json`, `tokio`, `tracing`

---

## rekindle-voice

Voice chat pipeline: audio capture, Opus encoding, voice activity detection,
jitter buffering, mixing, and Veilid-based transport.

### Module Structure

```
src/
├── lib.rs                  VoiceEngine public API
├── error.rs                Voice error types
├── capture.rs              Microphone input via cpal (dedicated thread)
├── playback.rs             Speaker output via cpal (dedicated thread)
├── codec.rs                Opus encode/decode (48kHz mono)
├── vad.rs                  Energy-based voice activity detection
├── jitter.rs               Adaptive jitter buffer for network variance
├── mixer.rs                Multi-participant audio stream mixing
└── transport.rs            Veilid-based voice packet send/receive
```

### Voice Pipeline

```
┌───────────┐    ┌─────────┐    ┌─────┐    ┌────────┐    ┌───────────┐
│  cpal     │    │ Accumu- │    │ VAD │    │  Opus  │    │ Transport │
│  capture  │───→│  late   │───→│     │───→│ encode │───→│  send()   │
│ (thread)  │    │ frames  │    │     │    │        │    │           │
└───────────┘    └─────────┘    └─────┘    └────────┘    └───────────┘

┌───────────┐    ┌────────┐    ┌────────┐    ┌───────┐    ┌──────────┐
│ Transport │    │  Opus  │    │ Jitter │    │ Mixer │    │  cpal    │
│ receive() │───→│ decode │───→│ buffer │───→│       │───→│ playback │
│           │    │        │    │        │    │       │    │ (thread) │
└───────────┘    └────────┘    └────────┘    └───────┘    └──────────┘
```

### Threading Model

`cpal::Stream` is `!Send` on macOS. Audio capture and playback streams live on
dedicated OS threads and communicate with the async runtime via `mpsc` channels.
The voice send loop runs as a spawned Tokio task.

### Transport

Voice packets use `SafetySelection::Unsafe` for direct UDP-like delivery over
Veilid, bypassing privacy routing to minimize latency. The `VoiceTransport`
`connect()` and `disconnect()` methods are synchronous (not async).

### Key Types

| Type | Description |
|------|-------------|
| `VoiceEngine` | Central controller: capture, playback, mute/deafen state |
| `OpusCodec` | Encoder/decoder wrapper (48kHz, mono, 20ms frames) |
| `VoiceActivityDetector` | Energy-threshold VAD with configurable sensitivity |
| `JitterBuffer` | Adaptive buffer compensating for network timing variance |
| `AudioMixer` | Mixes multiple decoded participant streams |
| `VoiceTransport` | Veilid-backed packet send/receive (unsafe safety selection) |

### External Dependencies

`opus`, `cpal`, `tokio`, `tracing`, `bytes`, `thiserror`

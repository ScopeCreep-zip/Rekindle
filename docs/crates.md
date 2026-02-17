# Rust Crate Reference

Rekindle's business logic is split across five pure Rust crates with zero Tauri
dependency, plus a community server daemon. This separation ensures the
protocol, cryptography, game detection, and voice subsystems can be tested
independently and reused outside the Tauri shell.

## Workspace Structure

```
crates/
├── rekindle-protocol/      Veilid networking, DHT, Cap'n Proto
├── rekindle-crypto/        Ed25519 identity, Signal Protocol, group encryption
├── rekindle-game-detect/   Cross-platform game detection
├── rekindle-voice/         Opus codec, audio I/O, VAD, transport
└── rekindle-server/        Community hosting daemon (child process)
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
│   ├── envelope.rs         MessageEnvelope, MessagePayload, InviteBlob, CommunityRequest/Response/Broadcast
│   ├── sender.rs           Outbound delivery via app_message + app_call (8s timeout RPC)
│   └── receiver.rs         Inbound message dispatch and verification
└── dht/
    ├── mod.rs              DHTManager, record operations
    ├── profile.rs          User profile record (8 subkeys)
    ├── presence.rs         Presence data read/write
    ├── friends.rs          Friend list DHT record
    ├── community.rs        Community DHT records (SMPL multi-writer)
    ├── channel.rs          Channel message batches (linked-list records, max 50/batch)
    ├── conversation.rs     Conversation DHT records (encrypted with DH shared secret)
    ├── account.rs          Account record (encrypted with identity secret)
    ├── mailbox.rs          Mailbox DHT record (route blob inbox)
    ├── short_array.rs      DHTShortArray (ordered collection, max 255 elements)
    └── log.rs              DHTLog (append-only log across DHT records)
```

### Key Types

| Type | Description |
|------|-------------|
| `RekindleNode` | Veilid node lifecycle (start, attach, shutdown) |
| `DHTManager` | Owns a `RoutingContext`, performs all DHT read/write/watch operations |
| `RoutingManager` | Allocates/maintains private routes, imports peer routes with 90s TTL cache |
| `PeerManager` | Peer address resolution (public key → route blob → RouteId) |
| `MessageEnvelope` | Serialized wrapper with Ed25519 signature for all application messages |
| `MessagePayload` | Typed payload enum: DirectMessage, ChannelMessage, FriendRequest/Accept/Reject, TypingIndicator, ProfileKeyRotated, PresenceUpdate |
| `InviteBlob` | Ed25519-signed invite with public key, display name, route info, prekey bundle |
| `CommunityRequest` | RPC request enum (22 variants): Join, SendMessage, Kick, Ban, CreateRole, etc. |
| `CommunityResponse` | RPC response enum: Ok, Joined, Messages, MEK, ChannelCreated, Error, etc. |
| `CommunityBroadcast` | Push broadcast enum: NewMessage, MEKRotated, MemberJoined/Removed, RolesChanged, etc. |
| `DHTLog` | Append-only log spanning multiple DHT records (spine + segments) |
| `DHTShortArray` | Ordered collection with O(1) remove via logical index map (max 255) |

### External Dependencies

`veilid-core`, `capnp`, `capnpc` (build), `tokio`, `tracing`, `thiserror`, `hex`, `base64`, `rekindle-crypto`

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
├── keychain.rs             Key storage trait (Stronghold abstraction), vault/key constants
├── dht_crypto.rs           DhtRecordKey: account key (HKDF from secret), conversation key (HKDF from DH shared secret), XChaCha20-Poly1305 encrypt/decrypt
├── group/
│   ├── mod.rs              Group encryption exports
│   ├── media_key.rs        MEK generation, AES-256-GCM encrypt/decrypt
│   └── pseudonym.rs        Community pseudonym derivation (HKDF-SHA256 → unlinkable Ed25519 per community)
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
| `Identity` | Ed25519 keypair with derived X25519 key, sign/verify, public key hex |
| `SignalSessionManager` | Manages Signal sessions for all peers (X3DH + Double Ratchet) |
| `PreKeyBundle` | Public keys published to DHT for session establishment |
| `MediaEncryptionKey` | AES-256-GCM symmetric key for community channels (with generation tracking) |
| `DhtRecordKey` | Symmetric encryption key for DHT records (account, conversation) |
| `Keychain` | Trait abstracting key storage (vault constants, key name helpers) |
| `derive_community_pseudonym()` | HKDF-SHA256 deterministic Ed25519 key per community (unlinkable) |

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

`ed25519-dalek`, `x25519-dalek`, `aes-gcm`, `chacha20poly1305`, `hkdf`, `sha2`,
`rand`, `zeroize`, `serde`, `thiserror`

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
| `DetectedGame` | Detected game: ID, name, process name, start timestamp |
| `list_process_names()` | Platform-specific process enumeration function (in `platform/mod.rs`) |

### External Dependencies

`sysinfo`, `serde`, `serde_json`, `tokio`, `tracing`

---

## rekindle-voice

Voice chat pipeline: audio capture, Opus encoding, voice activity detection,
jitter buffering, mixing, and Veilid-based transport.

### Module Structure

```
src/
├── lib.rs                  VoiceEngine public API (VoiceConfig, start/stop capture/playback)
├── error.rs                Voice error types
├── capture.rs              Microphone input via cpal (dedicated OS thread, mpsc bridge)
├── playback.rs             Speaker output via cpal (dedicated OS thread, VecDeque ring buffer)
├── codec.rs                Opus encode/decode (48kHz, VoIP mode, 32kbps, FEC enabled)
├── audio_processing.rs     AudioProcessor: RNNoise denoising + AEC3 echo cancellation + VAD
├── jitter.rs               Adaptive jitter buffer (BTreeMap by sequence, initial fill delay)
├── mixer.rs                Multi-participant audio stream mixing (per-participant volume, soft clamp)
└── transport.rs            Veilid-based voice packet send/receive (bincode serialized)
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
| `VoiceEngine` | Central controller: capture, playback, mute/deafen, device selection |
| `VoiceConfig` | Configuration: sample rate, channels, frame size, jitter buffer, VAD threshold, noise/echo flags |
| `OpusCodec` | Encoder/decoder (48kHz mono, VoIP mode, 32kbps, in-band FEC, 10% loss) |
| `AudioProcessor` | RNNoise denoising + AEC3 echo cancellation + energy-based VAD |
| `JitterBuffer` | Adaptive buffer with initial fill delay (BTreeMap by sequence) |
| `AudioMixer` | Mixes multiple decoded participant streams with per-participant volume |
| `VoiceTransport` | Veilid-backed packet send/receive (unsafe safety selection, bincode) |

### External Dependencies

`opus`, `cpal`, `dasp_sample`, `nnnoiseless`, `aec3`, `tokio`, `tracing`, `bytes`,
`thiserror`, `bincode`, `veilid-core`

---

## rekindle-server

Community hosting daemon. Runs as a child process spawned by the Tauri app when
a user owns communities. Handles community RPC (join, messaging, moderation),
MEK management, and member state.

### Module Structure

```
src/
├── main.rs                 Binary entry point
├── community_host.rs       Community hosting logic
├── db.rs                   SQLite database for server state
├── ipc.rs                  IPC communication with parent Tauri process
├── mek.rs                  MEK generation, rotation, distribution
├── rpc.rs                  RPC protocol handler (CommunityRequest → CommunityResponse)
└── server_state.rs         Server state management
```

### Key Behavior

The server process is:
- Spawned as a child process when a user creates or owns communities
- Health-checked every 30s by `server_health_service` in the Tauri app
- Automatically restarted if it becomes unresponsive (2 failures, 120s cooldown)
- Handles `CommunityRequest` RPC via Veilid `app_call`
- Broadcasts `CommunityBroadcast` events to community members via `app_message`

### External Dependencies

`veilid-core`, `rusqlite`, `tokio`, `serde`, `serde_json`, `tracing`,
`rekindle-protocol`, `rekindle-crypto`

# Network Protocol

Rekindle communicates over the Veilid peer-to-peer network. There is no central
server. All messages are end-to-end encrypted, signed, and delivered through
Veilid's `app_message` routing or written to SMPL DHT records. Distributed
state is stored in Veilid DHT records.

## Identity Model

Each user's identity is an Ed25519 keypair generated locally on first run. The
public key serves as the user's permanent address. There are no usernames,
passwords, or email addresses. The private key never leaves the device.

```
Identity creation:
  1. Generate Ed25519 keypair
  2. Derive X25519 key for Diffie-Hellman
  3. Store private keys in Stronghold vault (encrypted by passphrase)
  4. Publish public key + display name to DHT profile record
  5. Allocate a Veilid private route for receiving messages
  6. Publish a deterministic mailbox DHT record with the current route blob
  7. Publish an encrypted account record (contacts/chats/invites references)
```

## 1:1 Message Flow (Friends)

```
┌───────────┐                                              ┌───────────┐
│  Sender   │                                              │ Receiver  │
│           │                                              │           │
│ plaintext │                                              │ plaintext │
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│  Signal   │                                              │  Signal   │
│  encrypt  │                                              │  decrypt  │
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│ Envelope  │                                              │ Envelope  │
│ + Ed25519 │                                              │ verify    │
│ signature │                                              │ signature │
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│ Veilid    │    safety route    ┌─────────┐  private     │ Veilid    │
│app_message├──────────────────→│  Veilid │  route ─────→│ callback  │
│           │   (sender hidden) │ Network │(rcvr hidden)  │           │
└───────────┘                    └─────────┘               └───────────┘
```

Both sender and receiver privacy is protected. Safety routes hide the sender's
IP by routing through multiple Veilid nodes. Private routes hide the
receiver's IP in the same manner.

## Message Lifecycle (Friend DM)

| Stage | Layer | Operation |
|-------|-------|-----------|
| 1. Compose | Frontend | User types message, invokes `send_message` command |
| 2. Encrypt | rekindle-crypto | Signal Protocol Double Ratchet encryption (1:1) |
| 3. Sign | rekindle-codec | Build `MessageEnvelope`, sign over (timestamp ‖ nonce ‖ payload) with Ed25519 |
| 4. Serialize | rekindle-codec | bincode-serialize the envelope |
| 5. Send | rekindle-protocol | Look up peer's route blob, import route, `app_message()` |
| 6. Receive | services/veilid::dispatch | `VeilidUpdate::AppMessage` callback dispatches to `message_service` |
| 7. Decrypt | rekindle-crypto | Verify signature, Signal decrypt |
| 8. Store & Display | src-tauri | Insert into SQLite, emit `ChatEvent::MessageReceived` |

## MessageEnvelope (Wire Format)

All friend-to-friend messages are wrapped in a `MessageEnvelope`:

```rust
pub struct MessageEnvelope {
    pub sender_key: Vec<u8>,    // Ed25519 public key (32 bytes)
    pub timestamp: u64,         // Unix milliseconds
    pub nonce: Vec<u8>,         // Unique nonce (dedup + ordering)
    pub payload: Vec<u8>,       // Encrypted body
    pub signature: Vec<u8>,     // Ed25519 over (timestamp || nonce || payload)
}
```

Payload type discrimination uses an internally tagged serde enum
(`#[serde(tag = "type")]`).

### MessagePayload Variants

The `MessagePayload` enum (in `rekindle-protocol::messaging::envelope`)
covers all 1:1 traffic between friends — DMs, friend-request handshakes,
DM/group-DM invites, Strand Relay, Mobile Push Relay, presence cache, etc.

| Variant | Purpose |
|---------|---------|
| `DirectMessage` | 1:1 encrypted chat message (with optional reply) |
| `ChannelMessage` | Legacy community channel message (replaced by gossip) |
| `TypingIndicator` | Ephemeral typing state |
| `FriendRequest` | Initial friend contact (includes PreKeyBundle, profile/route/mailbox info, optional invite_id) |
| `FriendAccept` | Accept with PreKeyBundle, ephemeral key, and prekey IDs |
| `FriendReject` | Rejection notification |
| `FriendRequestReceived` | Lightweight ACK confirming delivery (not acceptance) |
| `ProfileKeyRotated` | Notify remaining friends of new DHT profile key after block/unfriend |
| `PresenceUpdate` | Inline presence (status, optional GameInfo) — fallback when DHT watch fails |
| `Unfriended` / `UnfriendedAck` | Notify peer of unfriending and confirm receipt |
| `RelayOffer` / `RelayOfferAck` / `RelayWithdraw` | Strand Relay (§13.2): friend volunteers to forward |
| `RelayEnvelope` | Strand Relay (§13.3): forwards an inner opaque envelope to a target |
| `DmInvite` / `DmAccept` / `DmDecline` / `DmLeave` | 2-party DM lifecycle (§27.1) |
| `GroupDmInvite` | Group DM with per-recipient X25519-wrapped MEK (§27.2) |
| `RegisterPushRelay` / `UnregisterPushRelay` / `WakeNotify` | Mobile Push Relay (§17.3 Tier 3) |
| `StatusRequest` / `StatusResponse` | Strand Relay presence cache (§13.5 — "social CDN") |

### Invite System

Friends can be added via Ed25519-signed `InviteBlob` payloads:

```rust
pub struct InviteBlob {
    pub public_key: String,
    pub display_name: String,
    pub mailbox_dht_key: String,
    pub profile_dht_key: String,
    pub route_blob: Vec<u8>,
    pub prekey_bundle: Vec<u8>,
    pub invite_id: Option<String>,  // Correlation token for outgoing-invite tracking
    pub signature: Vec<u8>,
}
```

- `create_invite_blob()` → sign blob with identity key
- `verify_invite_blob()` → verify Ed25519 signature
- Invites are encoded as `rekindle://invite/{base64url-blob}` deep links
- The `outgoing_invites` SQLite table tracks each issued invite by `invite_id`,
  status, and timestamps so the buddy list can show pending invitations

### Direct Messages and Group DMs (Architecture §27)

DMs are not friends — they are SMPL DHT records with `o_cnt: 0` (Schwarzschild
schema, no creator subkeys), exactly two member subkeys, and a MEK derived
deterministically via X25519 ECDH between the two identity keys. There is no
key-exchange round-trip — both peers compute the same MEK from each other's
identity public key and their own private key.

Group DMs (§27.2) carry the MEK wrapped per recipient with X25519, since ECDH
is pairwise.

The `rekindle-dm` crate is pure logic. The `src-tauri/services/dm/` layer
wires it to Veilid (record creation, app_message invitations) and SQLite
(local message history).

## Community Architecture (v2.0 Flat Governance)

Communities use **flat SMPL governance** — there is no coordinator process and
no privileged nodes. Every member is a full peer. Distributed state lives in
SMPL records with `o_cnt: 0` (the creation keypair is discarded after genesis).

### Three-Path Delivery

Every community write follows three parallel paths:

1. **SMPL write** (durable) — the writer appends to the appropriate channel
   SMPL record so offline peers see it on next login. Tier 3
   (`rekindle-records`).
2. **Gossip mesh** (fast) — D-fanout broadcast over `app_message` for sub-second
   delivery to online peers. Tier 5 (`rekindle-gossip`).
3. **Watch / inspect** (consistent) — `watch_dht_values` plus periodic
   `inspect_dht_record` polling reconciles late joiners and detects gaps.

### Adaptive Gossip Fan-out

`rekindle_gossip::mesh::fanout_degree()`:

| Members | D |
|---------|---|
| ≤ 20 | N − 1 (direct mesh) |
| 21 – 60 | 6 |
| 61 + | 8 |

### Envelope Types

Community traffic uses a `SignedEnvelope` (built/verified by `rekindle-codec`)
that wraps a `CommunityEnvelope` payload. Payload variants include
`ChatMessage`, `VoicePacket`, `ControlMessage` (governance entry, mod actions),
`PresenceUpdate`, `ChannelTyping`, `SoundboardPlay`, video frames /
acknowledgements / topology change, link previews, etc. All envelopes are
signed with the sender's per-community pseudonym key and carry Lamport
timestamps for causal ordering.

### CRDT Governance Merge

`rekindle-governance::merge::merge()` is a pure function: given the set of
`GovernanceEntry` variants from all member subkeys, sort by
`(lamport, author_pseudonym)` and apply deterministic merge rules to produce a
`GovernanceState`. Every peer running the same merge on the same entries
produces an identical result.

Permissions are reader-validated: each peer checks the merged state to decide
whether to accept an incoming envelope (e.g., did this pseudonym have
`SEND_MESSAGES` in this channel at that lamport?).

### MEK Distribution

Each channel has its own MEK (per-channel, not per-community). MEKs are
distributed via the SMPL member registry's MEK vault (encrypted per-member
slot). Rotation uses the deterministic rotator
(`blake3(departed_pseudonym ‖ self_pseudonym)` — lowest hash wins) to pick the
peer responsible for re-wrapping.

Full architecture: [`../architecture/communities.md`](../architecture/communities.md) (chiral-network v2.0).

## Cap'n Proto Schema Catalog

| Schema File | Top-level structs |
|-------------|-------------------|
| `message.capnp` | `MessageEnvelope`, `ChatMessage`, `Attachment` |
| `identity.capnp` | `UserProfile`, `PreKeyBundle` |
| `presence.capnp` | `PresenceUpdate`, `GameStatus` |
| `friend.capnp` | `FriendRequest`, `FriendList`, `FriendEntry` |
| `community.capnp` | `Community`, `Channel`, `Role`, `PermissionOverwrite` |
| `voice.capnp` | `VoiceSignaling` |
| `conversation.capnp` | `ConversationHeader` |
| `account.capnp` | `AccountHeader`, `ContactEntry`, `ChatEntry` |

Generated Rust modules are included at the `rekindle-protocol` crate root via
`pub mod foo_capnp { include!(...) }` so generated `crate::<schema>_capnp`
paths resolve.

## Veilid Primitives Used

| Primitive | Usage |
|-----------|-------|
| `app_message(target, data)` | Fire-and-forget delivery to a `RouteId` |
| `app_call(target, data)` | Request-response delivery (DM invite handshake) |
| `create_dht_record(schema)` | Create a new DHT record (DFLT or SMPL) |
| `open_dht_record(key, keypair)` | Open existing record with optional write access |
| `set_dht_value(key, subkey, data)` | Write to a subkey of an owned record |
| `get_dht_value(key, subkey, force)` | Read a subkey (force=true bypasses cache) |
| `inspect_dht_record(key, subkeys)` | Read seq numbers without fetching data |
| `watch_dht_values(key, subkeys)` | Subscribe to change notifications |
| `close_dht_record(key)` | Release a record handle |
| `new_custom_private_route(stability, sequencing)` | Allocate a private route |
| `import_remote_private_route(blob)` | Import a peer's route blob for sending |
| `RoutingContext` | Scoped handle for all DHT and message operations |

## DHT Record Layouts

### User Profile Record (DFLT, 8 subkeys)

| Subkey | Content |
|--------|---------|
| 0 | Display name (UTF-8) |
| 1 | Status message (UTF-8) |
| 2 | Status enum: `online`, `away`, `busy`, `offline`, `invisible` |
| 3 | Avatar (WebP, raw bytes) |
| 4 | Game info (Cap'n Proto `GameStatus`) |
| 5 | PreKeyBundle for Signal session establishment |
| 6 | Private route blob (for receiving `app_message`) |
| 7 | Metadata (reserved) |

Friends watch each other's DHT records via `watch_dht_values`. When a subkey
changes, Veilid delivers a `VeilidUpdate::ValueChange` to the watcher, which
the `presence_service` processes into a `PresenceEvent`.

When a watch fails to establish (Veilid GitLab #377), the friend is added to
`AppState.unwatched_friends` and the sync service polls with `force_refresh=true`.

### Mailbox DHT Record

Each user publishes a mailbox DHT record created with their identity keypair
(deterministic key). It contains only the user's current Veilid route blob,
providing a fallback for peers when the profile record's subkey 6 is stale.

### Conversation DHT Records (Per-Friend)

Each friend pair maintains two DHT records — one owned by each side. Records
are encrypted with `DhtRecordKey::derive_conversation_key()` using the X25519
DH shared secret between the two parties. Each record contains a child
`DHTLog` for append-only message history.

### Account Record (Cross-Device Recovery)

Encrypted with `DhtRecordKey::derive_account_key()` from the user's Ed25519
secret. Holds three child `DHTShortArray` references for contacts, chats, and
invitations. The cross-device sync layer (`rekindle-sync` +
`services/cross_device_sync`) extends this with a personal sync record
(`personal_sync_record_key` on the `identity` row) that holds device list,
read state, preferences, and pairing state.

### Community Records (SMPL `o_cnt: 0`)

All community records share the universal v2.0 SMPL schema (255 member slots):

- **Governance record** — every member writes their `GovernanceEntry` history
  to their own subkey. Reader merges all subkeys via the CRDT.
- **Member registry record** — claim-and-hold slot model. Members claim a
  free subkey on join; segments are added when 255 fill up (Plate Gates).
- **Channel records** — one SMPL record per channel for message persistence;
  same 255-subkey layout.

Plate Gates (architecture §15) handle scaling beyond 255 members by adding
fractal SMPL segments. `CommunityState.my_segment_index` tracks which
segment the user lives in.

## Offline Message Handling

When a peer is unreachable (no valid route), 1:1 messages are queued in the
`pending_messages` SQLite table. The `sync_service` retries delivery every
30 seconds, incrementing `retry_count` on each attempt. Messages are discarded
after 20 failed retries.

Ephemeral messages (typing indicators) are not queued. Friend requests,
accepts, rejects, and unfriend notifications are queued for reliable delivery.

For community channel messages, the SMPL write is the durable record — a peer
that comes online later catches up by reading the channel SMPL subkeys
directly.

## Strand Relay (Architecture §13)

When two friends cannot reach each other directly (NAT / route churn), a
mutual friend can volunteer to forward. The relayer never sees plaintext —
they receive a `RelayEnvelope` containing an opaque inner payload addressed
to the target. The friend volunteers via `RelayOffer` (acknowledged with
`RelayOfferAck`) and can withdraw at any time with `RelayWithdraw`.
`StatusRequest` / `StatusResponse` form a presence cache so the requester
can short-circuit DHT route lookup as well (the "social CDN" pattern).

## Mobile Push Relay (Architecture §17.3 Tier 3)

For mobile clients that cannot keep a Veilid connection open in the
background, a headless `veilid-server` push relay watches a list of DHT
record keys on the client's behalf and sends content-free wake signals
(`{"t":"wake"}`) via FCM/APNs. Only the mobile client can decrypt the actual
record contents — the relay sees only that *some* registered record fired.

## Community Pseudonyms

Users participate in communities under unlinkable pseudonyms.
`derive_community_pseudonym()` (in `rekindle-crypto::group::pseudonym`) uses
HKDF-SHA256 from the user's master secret and community ID to derive a
unique Ed25519 keypair per community:

- Same user gets different pseudonyms in different communities
- Same user always gets the same pseudonym in the same community
- No correlation between a user's pseudonyms across communities

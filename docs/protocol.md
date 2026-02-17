# Network Protocol

Rekindle communicates over the Veilid peer-to-peer network. There is no central
server. All messages are end-to-end encrypted, serialized with Cap'n Proto, and
delivered through Veilid's `app_message` routing. Distributed state is stored in
Veilid DHT records.

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
```

## Node-to-Node Message Flow

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
│ Cap'n Proto│                                             │ Cap'n Proto│
│ serialize │                                              │ deserialize│
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│ Veilid    │    safety route    ┌─────────┐  private     │ Veilid    │
│ app_message├──────────────────→│  Veilid │  route  ────→│ callback  │
│           │   (sender hidden) │ Network │(rcvr hidden)  │           │
└───────────┘                    └─────────┘               └───────────┘
```

Both sender and receiver privacy is protected. Safety routes hide the sender's
IP by routing through multiple Veilid nodes. Private routes hide the receiver's
IP in the same manner.

## Message Lifecycle

A message passes through eight stages from composition to display:

| Stage | Layer | Operation |
|-------|-------|-----------|
| 1. Compose | Frontend | User types message, invokes `send_message` command |
| 2. Encrypt | rekindle-crypto | Signal Protocol Double Ratchet encryption (1:1); MEK for channels (not yet wired) |
| 3. Sign | rekindle-crypto | Ed25519 signature over (timestamp ‖ nonce ‖ payload) for authenticity |
| 4. Serialize | rekindle-protocol | Cap'n Proto `MessageEnvelope` encoding |
| 5. Send | rekindle-protocol | Look up peer's route blob, import route, `app_message()` |
| 6. Receive | veilid_service | `VeilidUpdate::AppMessage` callback dispatches to `message_service` |
| 7. Decrypt | rekindle-crypto | Verify signature, Signal decrypt (1:1); channel messages currently plaintext |
| 8. Store & Display | src-tauri | Insert into SQLite, emit `ChatEvent::MessageReceived` |

## Message Envelope

All application messages are wrapped in a `MessageEnvelope` Cap'n Proto
structure:

```
MessageEnvelope:
  senderPublicKey:    Data     # Ed25519 public key of sender (32 bytes)
  timestamp:          UInt64   # Unix milliseconds
  nonce:              Data     # Unique message nonce (deduplication and ordering)
  payload:            Data     # Encrypted message body
  signature:          Data     # Ed25519 signature over (timestamp || nonce || payload)
```

Payload type discrimination is handled by the `MessagePayload` serde enum
(`#[serde(tag = "type")]`), not by a separate field.

### MessagePayload Types

The `MessagePayload` enum discriminates payload types:

| Variant | Purpose |
|---------|---------|
| `DirectMessage` | 1:1 encrypted chat message |
| `ChannelMessage` | Community channel message |
| `TypingIndicator` | Ephemeral typing state |
| `FriendRequest` | Initial friend contact with PreKeyBundle |
| `FriendAccept` | Accept with PreKeyBundle + Signal session info |
| `FriendReject` | Rejection notification |
| `ProfileKeyRotated` | Notify friends of new DHT profile key |
| `PresenceUpdate` | Inline presence (fallback for DHT watch failures) |

### Invite System

Friends can be added via Ed25519-signed invite blobs:

```
InviteBlob {
  public_key, display_name, mailbox_dht_key,
  profile_dht_key, route_blob, prekey_bundle, signature
}
```

- `generate_invite()` → sign blob with identity key
- `verify_invite_blob()` → verify Ed25519 signature
- `encode_invite_url()` / `decode_invite_url()` → base64url for `rekindle://invite/{blob}`

### Community Server Architecture

Communities use a client-server model within the P2P network. The community
owner runs a `rekindle-server` child process that acts as a message relay and
state manager.

**CommunityRequest** (24 RPC variants): Join, SendMessage, GetMessages, RequestMEK,
Leave, Kick, CreateChannel, DeleteChannel, RotateMEK, RenameChannel, UpdateCommunity,
Ban, Unban, GetBanList, CreateRole, EditRole, DeleteRole, AssignRole, UnassignRole,
SetChannelOverwrite, DeleteChannelOverwrite, TimeoutMember, RemoveTimeout, GetRoles

**CommunityResponse**: Ok, Joined, Messages, MEK, ChannelCreated, CommunityUpdated,
BanList, RoleCreated, RolesList, Error

**CommunityBroadcast** (push to all members): NewMessage, MEKRotated, MemberJoined,
MemberRemoved, RolesChanged, MemberRolesChanged, MemberTimedOut, ChannelOverwriteChanged

RPC uses Veilid `app_call()` with an 8-second timeout. Broadcasts use `app_message()`.

## Cap'n Proto Schema Catalog

| Schema File | Purpose |
|-------------|---------|
| `message.capnp` | `MessageEnvelope`, `DirectMessage`, `ChannelMessage` |
| `identity.capnp` | `IdentityRecord`, `PreKeyBundle` |
| `presence.capnp` | `PresenceRecord`, `GameInfo` |
| `friend.capnp` | `FriendRequest`, `FriendResponse`, `FriendListEntry` |
| `community.capnp` | `CommunityRecord`, `ChannelRecord`, `MemberRecord`, `RoleRecord` |
| `voice.capnp` | `VoiceSignaling`, `VoicePacket` |
| `conversation.capnp` | `ConversationRecord`, DHT-backed message history |
| `account.capnp` | `AccountRecord`, cross-device identity recovery |

Generated Rust modules are included at the crate root via
`pub mod foo_capnp { include!(...) }` in each crate's `lib.rs`.

## Veilid Primitives

| Primitive | Usage |
|-----------|-------|
| `app_message(target, data)` | Fire-and-forget delivery to a `RouteId` or `NodeId` |
| `app_call(target, data)` | Request-response delivery (used for community server RPC, 8s timeout) |
| `create_dht_record(schema)` | Create a new DHT record with DFLT or SMPL schema |
| `open_dht_record(key, keypair)` | Open an existing record (keypair for write access) |
| `set_dht_value(key, subkey, data)` | Write to a subkey of an owned record |
| `get_dht_value(key, subkey, force)` | Read a subkey (force=true bypasses cache) |
| `watch_dht_values(key, subkeys)` | Subscribe to change notifications on subkeys |
| `close_dht_record(key)` | Release a record handle |
| `new_custom_private_route(stability, sequencing)` | Allocate a private route for receiving messages |
| `import_remote_private_route(blob)` | Import a peer's route blob for sending |
| `RoutingContext` | Scoped handle for all DHT and message operations |

## DHT Profile Record Layout

Each user publishes a DHT record with 8 subkeys:

| Subkey | Content |
|--------|---------|
| 0 | Display name (UTF-8) |
| 1 | Status message (UTF-8) |
| 2 | Status enum: `online`, `away`, `busy`, `offline` |
| 3 | Avatar (WebP, raw bytes) |
| 4 | Game info (Cap'n Proto `GameInfo`) |
| 5 | PreKeyBundle for Signal session establishment |
| 6 | Private route blob (for receiving `app_message`) |
| 7 | Metadata (reserved) |

Friends watch each other's DHT records via `watch_dht_values`. When a subkey
changes, Veilid delivers a `VeilidUpdate::ValueChange` to the watcher, which
the `presence_service` processes into a `PresenceEvent`.

## Offline Message Handling

When a peer is unreachable (no valid route), messages are queued in the
`pending_messages` SQLite table. The `sync_service` retries delivery every
30 seconds, incrementing `retry_count` on each attempt. Messages are discarded
after 20 failed retries.

Ephemeral messages (typing indicators) are not queued. Friend requests, accepts,
and rejects are queued to ensure reliable delivery.

## Mailbox DHT Records

Each user publishes a mailbox DHT record created with their identity keypair
(deterministic key). It contains only the user's current Veilid route blob,
providing a fallback for peers when the profile record's subkey 6 is stale.

## Conversation DHT Records

Each friend pair maintains two DHT records — one owned by each side. Records
are encrypted with `DhtRecordKey::derive_conversation_key()` using the X25519
DH shared secret between the two parties. Each record contains a child `DHTLog`
for append-only message history.

## Community Pseudonyms

Users participate in communities under unlinkable pseudonyms. The
`derive_community_pseudonym()` function uses HKDF-SHA256 from the user's
master secret and community ID to deterministically derive a unique Ed25519
keypair per community. This ensures:

- Same user gets different pseudonyms in different communities
- Same user always gets the same pseudonym in the same community
- No correlation between a user's pseudonyms across communities

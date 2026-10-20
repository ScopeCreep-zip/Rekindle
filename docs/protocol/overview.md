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
  3. Store private keys in the vault (`rekindle-vault`, SQLCipher double-encrypted)
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
│ DmContent │                                              │ DmContent │
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│ Carrier   │                                              │ Carrier   │
│ encrypt   │                                              │ open      │
│ (ratchet /│                                              │ (ratchet /│
│ PreKeyInit│                                              │ PreKeyInit│
│ / sealed) │                                              │ / sealed) │
│     │     │                                              │     ▲     │
│     ▼     │                                              │     │     │
│ 'D' ‖     │                                              │ 'D' ‖     │
│ DmFrame   │                                              │ DmFrame   │
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
| 2. Encode | rekindle-codec | Build the kind's body (`DmText`, ...), wrap it in a `DmContent {contentHint, sentAtMs, payload}` (a `SealedContent` with a random nonce for pre-session kinds) |
| 3. Seal | rekindle-crypto, rekindle-db, rekindle-secrets | By the kind's carrier (below): step the Double Ratchet in one DB transaction (`rekindle_db::ratchet`), open a session with a PreKeyInit, or HPKE-seal and Ed25519-sign |
| 4. Serialize | rekindle-codec | Cap'n Proto-encode the `DmFrame` (`capnp_codec::direct`) and prefix the `'D'` tag |
| 5. Send | rekindle-transport | Look up peer's route blob, import route, `app_message()` (`operations::dm::send`) |
| 6. Receive | rekindle-transport | `rekindle_codec::ingress::classify` routes `'D'` to `operations::direct::frame`; an untagged datagram is dropped |
| 7. Verify & open | rekindle-transport, rekindle-crypto | Block list, friend gate for the kind, then the carrier's checks: signature, freshness and replay guard for sealed; ratchet decrypt; PreKeyInit session open |
| 8. Store & Display | rekindle-transport, host | The decrypted plaintext is written to `unprocessed` in the ratchet transaction, applied (insert into SQLite, emit `ChatEvent::MessageReceived`) and then deleted; rows left at a crash are replayed at unlock (D24) |

## DmFrame (Wire Format)

A 1:1 datagram is the ingress tag `'D'` followed by one packed Cap'n Proto
`DmFrame` (`schemas/direct.capnp`; codec in
`rekindle-codec::capnp_codec::direct`). Community gossip is tagged `'G'`
(`rekindle-codec::ingress`). An untagged datagram is dropped.

```
DmFrame {
  typeId  :UInt16     // the kind (type_id.rs); decides the body
  sender  :Data       // 32-byte Ed25519 identity (identity == address)
  body    :union { ratchet | preKeyInit | sealed }
}
```

The typeId names the kind and its carrier (`DmTypeId::carrier()`):

| Carrier | Body | Kinds |
|---------|------|-------|
| Ratchet | `{header {dh, pn, n}, ciphertext}` of a `DmContent` | DMs, edits, reactions, typing, unfriend, rotations, group DM, call signaling, DM video, Strand Relay, status |
| PreKeyInit | PQXDH init (base key, prekey ids, ML-KEM ciphertext) plus the first ratchet message | opens a session; the first payload is a `FriendAccept` or `SessionResetAccept` (or any ratchet kind) |
| Sealed | `{enc, ciphertext, signature}` of a `SealedContent {nonce, sentAtMs, payload}` | pre-session kinds: `FriendRequest`, `FriendReject`, `SessionResetRequest`/`Decline`, `RegisterPushRelay`/`UnregisterPushRelay`/`WakeNotify` |

**Ratchet.** The spec Double Ratchet (`rekindle-crypto::signal::{dr,record}`,
`SessionRecord`) with one record per peer in `ratchet_sessions`. The send
path steps it and commits in one DB transaction before the frame leaves.
The associated data binds both identities and the typeId.

**PreKeyInit.** Opens a session with PQXDH revision 3
(`signal/pqxdh/agreement.rs`: X25519 plus ML-KEM-768, with the signed and
one-time or last-resort prekeys in `signal_prekeys`) and carries its first
payload encrypted under it.

**Sealed.** HPKE Base (`rekindle-secrets::hpke`) to the recipient
identity's X25519 key, plus the sender's Ed25519 signature over
`DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖ ciphertext`. Binding the
recipient means a captured frame verifies only for the identity it was sent
to. The receiver rejects frames outside the freshness window (5 minutes
old, 60 seconds ahead) and drops a second copy of a `(sender, nonce)` it
already accepted (`operations::direct::replay`); a retry keeps the nonce and
re-seals with a fresh `sentAtMs`. Ratchet frames need no replay guard: a
used `(dh, n)` has no key left.

The daemon's framed community RPCs (`SignedPayload`, `app_call`) carry
the same recipient binding under `"rekindle-frame-sig-v3"`, plus the frame
`TypeId`, so signed bytes cannot be re-framed as another message type. No
1:1 message is framed that way: both tracks send 1:1 traffic as `'D'`
frames (plan E2.1), and E2.3 deleted `MessagePayload`, `MessageEnvelope`
and the framed DM kinds.

### DmFrame Kinds

| Kind (typeId) | Purpose |
|---------------|---------|
| `DmText` (0x0100), `DmEdit`/`DmDelete`/`DmReaction` (0x0110-0x0112) | 1:1 chat message (with optional reply and expiry), edits, deletes, reactions |
| `Typing` (0x0401) | Ephemeral typing state: the sender's backend sends the start, refreshes it every 3 s while the draft changes, and stops after 3 s idle; the receiver lapses an unrefreshed start after 5 s (`rekindle_events::typing`) |
| `FriendRequest` (0x0002) | Initial friend contact, sealed, by `app_call`; the reply is its receipt |
| `FriendAccept` (0x0101) | Accept; travels as the first payload of the PreKeyInit that opens the session |
| `FriendReject` (0x0102) | Rejection notification, sealed |
| `ProfileKeyRotated` (0x0105) | Notify remaining friends of new DHT profile key after block/unfriend |
| `IdentityRotated` (0x0108) | Identity moved to a new key; signed by both keys over `IDENTITY_ROTATED ‖ old ‖ new ‖ recipient`, sent over the old session (`operations::friend::identity_rotation`) |
| `Unfriended` / `UnfriendedAck` (0x0103/0x0104) | Notify peer of unfriending and confirm receipt |
| `SessionResetRequest` / `SessionResetAccept` / `SessionResetDecline` (0x0007-0x0009) | Start a new session; the accept is the first payload of the answering PreKeyInit |
| `RetryRequest` (0x0006) | Allocated at E2.3; body and sends arrive with E2.5 |
| `RelayOffer` / `RelayOfferAck` / `RelayWithdraw` (0x0130-0x0132), `RelayEnvelope` (0x0003) | Strand Relay (§13.2-13.3). The sender-side relay fallback was deleted at E2.3 and returns at E5.5 |
| `GroupDmInvite` / `GroupDmAccept` / `GroupDmDecline` / `GroupDmLeave` (0x0120-0x0123) | Group DM lifecycle with per-recipient X25519-wrapped MEK (§27.2) |
| `Call*` (0x0200-0x0206), `GroupCall*` (0x0210-0x0214) | Direct and group call signaling |
| `DmVideoFragment` (0x0220) | DM video |
| `RegisterPushRelay` / `UnregisterPushRelay` / `WakeNotify` (0x0310-0x0312) | Mobile Push Relay (§17.3 Tier 3), sealed |
| `StatusRequest` / `StatusResponse` (0x0140/0x0141) | Strand Relay presence cache (§13.5 "social CDN") |

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

Each channel has its own MEK (per-channel, not per-community). The MEK is
**never written to the DHT**. The rotator wraps one copy per recipient
(X25519 ECDH from its own pseudonym) and delivers each by `app_call`
over that member's private route.

Rotation uses the deterministic rotator
(`blake3(departed_pseudonym ‖ self_pseudonym)` — lowest hash wins) to pick the
peer responsible for re-wrapping, cascading to the next candidate when the
primary does not deliver. An operator-requested rotation skips the cascade
and rotates in place (`rekindle_mek_rotation::rotate_mek_on_request`),
stamping its election rank so two admins rotating at one generation still
converge.

Full architecture: [`../architecture/communities.md`](../architecture/communities.md) (chiral-network v2.0).

## Cap'n Proto Schema Catalog

| Schema File | Top-level structs |
|-------------|-------------------|
| `direct.capnp` | `DmFrame`, `DmContent`, `SealedContent`, the 1:1 kind bodies |
| `identity.capnp` | `PreKeyBundle` |
| `presence.capnp` | `PresenceUpdate`, `GameStatus` |
| `friend.capnp` | `FriendRequest`, `FriendList`, `FriendEntry` |
| `community.capnp` | `Community`, `Channel`, `Role`, `PermissionOverwrite` |
| `voice.capnp` | `VoiceSignaling` |
| `account.capnp` | `AccountHeader`, `ContactEntry`, `ChatEntry` |

Generated Rust modules are included at the `rekindle-protocol` crate root via
`pub mod foo_capnp { include!(...) }` so generated `crate::<schema>_capnp`
paths resolve.

## Veilid Primitives Used

| Primitive | Usage |
|-----------|-------|
| `app_message(target, data)` | Fire-and-forget delivery to a `RouteId`. Used for DMs, friend-add, call signaling (W13/W16), DM invites, and voice/video frames. DMs that need retry go through the `pending_messages` queue (E2.5 replaces it with the send log); the W16 `pending_envelopes` layer was never wired and was deleted (plan E2.3). |
| `app_call(target, data)` | Request-response delivery. Reserved for operations that genuinely need an inline reply: MEK delivery, file chunk transfer, bootstrap bundles. **Not used for call signaling or DM invites** — Wave 13/16 superseded those with `app_message` to avoid `app_call`'s connection-table exhaustion under bootstrap, ~25 s collapse under network churn, and signaling-vs-DM traffic-shape leakage. See `docs/architecture/voice.md` and the `.claude/docs/rekindle-communities-architecture.md` §10.10 / §27 for the design rationale. |
| DHT record calls (`create`/`open`/`close_dht_record`, `get`/`set_dht_value`, `inspect_dht_record`, `watch_dht_values`, `cancel_dht_watch`) | Only through the record pool, `rekindle_protocol::dht::pool::RecordPool` (one per session, both tracks). It holds each record's lease, sticky writer and watch; serializes writes per subkey; re-pushes missed writes of records held writable and persists them to the `dht_outbox` table at logout; closes ended sessions' records through the node's `RecordCloser`, after their calls finish, never aborting one mid-commit; and rehydrates community records on a jittered keepalive. A multi-writer read consults an index (one inspect over the writer set) before fetching (plan C7.12). |
| `new_private_route` / `new_custom_private_route` / `release_private_route` | Only through `rekindle_route::own_routes::OwnRoutes` (one per node, over `rekindle_protocol::own_routes::VeilidRouteAllocator`): a General route and a Media route (LowLatency, PreferUnordered; never substituted by General). Allocated at network readiness, single-flight per class, `TryAgain` retried with backoff; a route Veilid reports dead is forgotten, never released, and reallocated; logout releases and renews, so no route is shared between identities. The relay feature owns its dedicated relay routes (`services/relay/offer.rs`). |
| `import_remote_private_route(blob)` | Only through `RouteImports` (one per node). A dead or failed peer route is forgotten, never released: Veilid releases a dead remote route itself. |
| Node capabilities | App nodes disable `DHTV` (they do not store records for others; veilid #492). |
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
to the target (the sender-side fallback returns at E5.5). The friend volunteers via `RelayOffer` (acknowledged with
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

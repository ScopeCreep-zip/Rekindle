# Data Storage and Serialization

Rekindle uses three storage backends, each serving a distinct purpose. SQLite
stores local state and message history. Stronghold encrypts private keys at
rest. Veilid DHT provides distributed storage for profile data, presence, and
community state.

## Storage Backend Summary

| Backend | Scope | Contents |
|---------|-------|----------|
| SQLite | Local device | Identity, friends, messages, communities, Signal sessions, DMs, sync state, analytics |
| Stronghold | Local device | Ed25519/X25519 private keys, Signal keying material, MEKs |
| Veilid DHT | Distributed | Profile, presence, mailbox, friend list, community governance / registry / channels, account record, personal sync record |
| Filesystem | Local device | Lost Cargo per-community chunk cache (`<app_data>/file_cache/<community_id>/...`) |

## SQLite Schema

The database file is stored at `{app_config_dir}/rekindle.db`. All tables
are defined in `src-tauri/migrations/001_init.sql`. The current
`SCHEMA_VERSION` (in `src-tauri/src/db.rs`) is **56** — bump it when the SQL
file changes.

### Identity and Friends

- **`identity`** — local user identity. One row per identity (multi-account).
  Includes Ed25519 public key, display name, DHT record keys for profile /
  friend list / account / mailbox / personal sync, optional avatar BLOB, and
  a stable per-device `device_id` (random 16-byte UUID hex) used for the
  cross-device DeviceList without exposing more PII.
- **`friend_groups`** — collapsible buddy list groupings (e.g., "Gaming").
- **`friends`** — friend list with per-identity scoping. Tracks display name,
  nickname override, group, DHT record key for presence watching, last-seen
  timestamp, cached avatar, conversation DHT keys (local + remote), mailbox
  key, and `friendship_state` (`accepted` / `pendingOut` / `removing`).

### Messaging

- **`messages`** — chat messages for both 1:1 DMs and community channels.
  Indexed by `(owner_key, conversation_id, timestamp)`. Carries
  `message_id` (channel messages), `mek_generation`, `lamport_ts`,
  `automod_blurred` flag, `forwarded_from_author`, and a `flags` bitmask.
  Dedup constraint on `(owner_key, conversation_id, conversation_type,
  sender_key, timestamp)`.
- **`pending_messages`** — offline retry queue (max 20 retries).
- **`pending_friend_requests`** — incoming friend requests awaiting user
  action; carries the requester's PreKeyBundle, profile/route/mailbox info,
  and optional `invite_id`.
- **`outgoing_invites`** — sent invites tracked by `invite_id` with status
  (`pending` / `responded` / `accepted` / `rejected` / `cancelled` / `expired`).
- **`blocked_users`** — blocked public keys per identity.
- **`message_delivery`** — per-message per-recipient delivery tracking
  (sending / delivered / failed) for community channel messages.

### Direct Messages (Architecture §27)

- **`dms`** — DM and group-DM conversations backed by SMPL records (`o_cnt:0`).
  Holds the SMPL `record_key`, `is_group` flag, initiator identity, our
  `my_subkey` index, JSON-encoded participant list, `slot_seed_hex`,
  `wrapped_mek_blob` (group DMs only), and `mek_generation`.
- **`dm_messages`** — local message log per DM (since one friend can have
  multiple unlinkable DM contexts).

### Communities

- **`communities`** — joined communities. Tracks DHT record keys (`manifest_key`,
  `member_registry_key`, `governance_key`), our pseudonym, current MEK
  generation, plate-gate `my_segment_index`, `my_subkey_index`,
  `lamport_clock`, and (legacy) coordinator fields.
- **`channels`** — channels within communities. Types: `text`, `voice`,
  `announcement`, `forum`, `stage`, `directory`, `media`, `events`, `dm`.
  Includes `topic`, `slowmode_seconds`, `nsfw` flag, per-channel
  `message_record_key`, `mek_generation`, `log_key`, and `my_sequence`
  (our send counter for gap detection).
- **`community_categories`** — channel categories.
- **`community_members`** — pseudonym registry per community. Carries roles,
  timeout, segment index, onboarding flag, and per-community profile fields
  (`bio`, `pronouns`, `theme_color`, `badges`, `avatar_ref`, `banner_ref`).
- **`community_member_leaves`** — append-only departure log for analytics
  (live `community_members` rows are deleted on leave).
- **`community_roles`** — role definitions. Includes `permissions` bitmask,
  `position`, `hoist`, `mentionable`, `self_assignable`, and optional
  `exclusion_group` (architecture §19.4).
- **`channel_overwrites`** — per-channel allow/deny bitmask overrides for a
  role or member.
- **`community_threads`** + **`thread_messages`** — forum-style sub-conversations
  (auto-archive timestamp, message count). `thread_messages` uses a synthetic
  INTEGER PK because FTS5 needs `content_rowid` to be a single INTEGER column;
  the logical key is preserved via UNIQUE constraint.
- **`community_events`**, **`event_rsvps`**, **`community_event_rsvps`** —
  scheduled events. `community_events` carries status (`scheduled` /
  `active` / `completed` / `cancelled`), JSON `recurrence_json` and
  `location_json`, `cover_image_ref`, and `max_attendees`. `event_rsvps`
  is the reader-aggregated view; `community_event_rsvps` is our local
  RSVP state mirrored into presence writes.
- **`channel_pins`** — pinned messages per channel.
- **`game_servers`** — community game-server favorites (label, address,
  game ID).
- **`community_invites`** — locally-cached invite codes for invites we
  created. Only `code_hash` lives in the DHT.
- **`peer_reliability`** — Mutual Aid topology metrics (architecture §14.5):
  per-peer per-community success/failure counts to weight gossip fan-out so
  high-reliability "ziplines" emerge organically.

### Signal Protocol & Trust

- **`trusted_identities`** — TOFU identity-key tracking with optional
  `verified` flag for out-of-band confirmation.
- **`signal_sessions`** — serialized Signal Protocol session state per peer.
- **`prekeys`** — Signal one-time and signed prekey storage.

### Notifications & Settings

- **`notification_preferences`** — per-channel/per-community level (0=All,
  1=Mentions, 2=Mute) plus optional `sound_ref` (community soundboard
  expression ID). Falls through channel → community → app-global.
- **`app_settings`** — global app settings: quiet hours window/timezone offset
  and the global Do Not Disturb toggle.

### Strand Relay (Architecture §13)

- **`strand_relay_offers`** — for each friend who volunteered to relay for us,
  their dedicated relay route blob and timestamp.
- **`strand_relay_volunteered`** — outbound: friends we've volunteered to
  relay for, with the dedicated route ID + blob (so we can revoke and route
  inbound `RelayEnvelope`s correctly).

### Mobile Push Relay (Architecture §17.3 Tier 3)

- **`push_relay_registrations`** — relay pseudonym, device push token,
  platform (`fcm` / `apns` / `self`), and the JSON list of record keys the
  relay is watching on this device's behalf.

### Analytics

- **`voice_session_events`** — append-only join/leave log for computing
  peak-concurrent-voice metrics per channel.

### Cross-Device Sync (Architecture §28.4)

- **`paired_devices`** — local cache mirror of the personal DFLT record's
  device list subkey. Tracks pairing/unpairing timestamps.
- **`channel_read_state`** — per-channel last-read Lamport. Reconciled into
  subkey 1 of the personal sync record.
- **`pending_pairings`** — short-lived rows holding the active pairing code
  and salt.

### Full-Text Search (Architecture §23)

Three FTS5 virtual tables with `unicode61 remove_diacritics 2` tokenizer
(no `porter` stemming because Rekindle is multilingual):

- `messages_fts` — content table `messages`, content_rowid `id`
- `thread_messages_fts` — content table `thread_messages`, content_rowid `id`
- `dm_messages_fts` — content table `dm_messages`, content_rowid `id`

Each has `AFTER INSERT/UPDATE/DELETE` triggers keeping the index in lock-step.

### Performance Indexes

- `idx_friends_group_id` on `friends(owner_key, group_id)`
- `idx_messages_conversation` on `messages(owner_key, conversation_id, timestamp)`
- `idx_messages_unread` on `messages(owner_key, conversation_id, is_read)` where `is_read = 0`
- `idx_messages_dedup` UNIQUE on `messages(owner_key, conversation_id, conversation_type, sender_key, timestamp)`
- `idx_messages_message_id` UNIQUE on `messages(owner_key, conversation_id, message_id)` where `message_id IS NOT NULL`
- `idx_channels_community_id` on `channels(owner_key, community_id)`
- `idx_community_members_community` on `community_members(owner_key, community_id, pseudonym_key)`
- `idx_pending_recipient` on `pending_messages(owner_key, recipient_key)`
- `idx_thread_messages_thread` on `thread_messages(owner_key, community_id, thread_id, timestamp)`
- `idx_dm_messages_record_ts` on `dm_messages(owner_key, record_key, timestamp)`
- `idx_member_leaves_recent` on `community_member_leaves(owner_key, community_id, left_at)`
- `idx_voice_session_events_channel` on `voice_session_events(owner_key, community_id, channel_id, occurred_at)`

## Schema Versioning

The schema version is tracked by a `SCHEMA_VERSION` constant in
`src-tauri/src/db.rs`. When the constant is incremented and the application
starts, the database detects a mismatch and drops all tables, recreating
them from `001_init.sql`.

Because SQLite, Stronghold, and Veilid DHT store interrelated state (friend
keys, DHT record keypairs, Signal sessions), a schema reset also triggers:

1. Deletion of all `.stronghold` files in the config directory
2. Removal of the `veilid/` local storage directory
3. Removal of the Lost Cargo file cache root

This ensures the four stores remain synchronized. Migration files are not
used because the schema is not yet stable for production.

## Database Access Pattern

The connection pool is `tokio_rusqlite::Connection` — a `tokio_rusqlite`
async wrapper over `rusqlite` running on a dedicated background thread. All
database access goes through `db_helpers::{db_call, db_call_or_default,
db_fire}`. Read-only state lookups go through `state_helpers`.

`rusqlite` 0.37 is used (not `sqlx`) to match `veilid-core`'s dependency on
the same version and avoid `libsqlite3-sys` build conflicts.

## Stronghold Vault

Each identity has a dedicated `.stronghold` file in the application config
directory. The vault is encrypted with a key derived from the user's
passphrase via Argon2id.

### Vault Namespaces

| Vault | Key | Purpose |
|-------|-----|---------|
| `identity` | `ed25519_private` | Ed25519 signing private key |
| `identity` | `x25519_private` | X25519 Diffie-Hellman private key |
| `signal` | `identity_keypair` | Signal Protocol identity keypair |
| `signal` | `signed_prekey` | Current signed prekey |
| `signal` | `prekey_batch` | Batch of one-time prekeys |
| `veilid` | `protected_store_key` | Veilid protected store encryption key |
| `communities` | `mek_{community_id}` | Per-community MEK (legacy) |
| `communities` | `channel_mek_{community_id}_{channel_id}` | Per-channel MEK |

Stronghold is accessed via `iota_stronghold` directly (not via the Tauri
Stronghold plugin) to allow per-identity snapshot files and configurable
Argon2 parameters for debug builds.

## DHT Record Layout

### User Profile Record (DFLT, 8 subkeys)

| Subkey | Content | Format |
|--------|---------|--------|
| 0 | Display name | UTF-8 |
| 1 | Status message | UTF-8 |
| 2 | Status enum | UTF-8 (`online`, `away`, `busy`, `offline`, `invisible`) |
| 3 | Avatar | WebP bytes |
| 4 | Game info | Cap'n Proto `GameStatus` |
| 5 | PreKeyBundle | Cap'n Proto `PreKeyBundle` |
| 6 | Route blob | Raw bytes (Veilid private route) |
| 7 | Metadata | Reserved (extensible header) |

### Friend List Record

A DHT record containing serialized friend list entries (Cap'n Proto
`FriendEntry` array). Used by peers to discover mutual friends and verify
friend relationships.

### Mailbox Record (DFLT, 1 subkey)

Each user publishes a mailbox DHT record created with their identity keypair
(deterministic key). Contains only the user's current Veilid private route
blob, providing a fallback when a friend's profile subkey 6 is stale.

| Subkey | Content |
|--------|---------|
| 0 | Route blob (raw bytes) |

### Account Record (DFLT, encrypted)

Private account record encrypted with `DhtRecordKey::derive_account_key()`
from the user's Ed25519 secret. Contains three child `DHTShortArray`
references for contacts, chats, and invitations.

### Personal Sync Record (DFLT, encrypted, architecture §28.4)

Lazy-created on first opt-in to multi-device. Subkeys hold the device list,
read state per channel, sync preferences, paired device public keys, and
sync manifest. Reconciled by the cross-device sync service.

### Conversation Record (DFLT, encrypted, per-friend pair)

Each friend pair maintains two DHT records — one owned by each side.
Records are encrypted with `DhtRecordKey::derive_conversation_key()` from
the X25519 DH shared secret. Each record contains a child `DHTLog` for
message history.

### Community Records (SMPL `o_cnt:0`, 255 subkeys — universal v2.0 schema)

All community records share the universal SMPL schema. The `o_cnt: 0`
property means the creation keypair is discarded after genesis — the
record has no privileged owner.

- **Governance record** — every member writes their `GovernanceEntry`
  history to their assigned subkey. Reader merges all subkeys via
  `rekindle-governance::merge::merge()`.
- **Member registry record** — claim-and-hold slot model. Members claim a
  free subkey on join. The MEK vault lives here too, encrypted per-slot.
- **Channel records** — one SMPL record per channel for message persistence.
  Same 255-subkey layout. Channel `message_record_key` is stored on the
  `channels` SQLite row.

When a community grows beyond 255 members, **Plate Gates** (architecture
§15) add fractal SMPL segments. `community_members.segment_index` and
`my_segment_index` track which segment hosts each member's slot.

### DM Record (SMPL `o_cnt:0`, 2 or N subkeys)

DMs are SMPL records with `o_cnt:0`, exactly two member subkeys for 2-party
DMs (or N for group DMs). The MEK is derived deterministically via X25519
ECDH between identity keys (no key-exchange round-trip). Group DMs wrap the
MEK per recipient.

## Lost Cargo File Cache (Architecture §28.9)

Per-community filesystem cache for chunked attachments managed by
`rekindle-files`. Path layout:

```
<app_data>/file_cache/<community_id>/<aa>/<full_attachment_hex>/<chunk_index>.bin
                                                                <chunk_index>.meta
```

Two-character fanout directory follows the Git object-store pattern.
Eviction runs synchronously after every `cache.insert()` until
`total_bytes ≤ budget`; pinned attachments are skipped. Pinned-attachment
IDs come from the merged `governance_state.pinned_attachments` set.

## Cap'n Proto Serialization

Network-facing structured data is serialized with Cap'n Proto for zero-copy
deserialization and schema evolution. Schemas live in `schemas/` and are
compiled at build time via `capnpc` in `rekindle-protocol`'s `build.rs`.
The generated Rust modules are included at the protocol crate root via
`pub mod foo_capnp { include!(...) }` so generated `crate::<schema>_capnp`
paths resolve.

| Schema | Top-level structs |
|--------|-------------------|
| `message.capnp` | `MessageEnvelope`, `ChatMessage`, `Attachment` |
| `identity.capnp` | `UserProfile`, `PreKeyBundle` |
| `presence.capnp` | `PresenceUpdate`, `GameStatus` |
| `friend.capnp` | `FriendRequest`, `FriendList`, `FriendEntry` |
| `community.capnp` | `Community`, `Channel`, `Role`, `PermissionOverwrite` |
| `voice.capnp` | `VoiceSignaling` |
| `conversation.capnp` | `ConversationHeader` |
| `account.capnp` | `AccountHeader`, `ContactEntry`, `ChatEntry` |

Bincode is used for the v2.0 community gossip envelope and DM payloads —
those are internal binary formats not intended for cross-language use.

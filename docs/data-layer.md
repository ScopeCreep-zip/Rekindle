# Data Storage and Serialization

Rekindle uses three storage backends, each serving a distinct purpose. SQLite
stores local state and message history. Stronghold encrypts private keys at
rest. Veilid DHT provides distributed storage for profile data and presence.

## Storage Backend Summary

| Backend | Scope | Contents |
|---------|-------|----------|
| SQLite | Local device | Identity, friends, messages, communities, Signal sessions |
| Stronghold | Local device | Ed25519/X25519 private keys, Signal keying material, MEKs |
| Veilid DHT | Distributed | Profile info, presence, route blobs, friend lists |

## SQLite Schema

The database file is stored at `{app_config_dir}/rekindle.db`. All tables
are defined in `src-tauri/migrations/001_init.sql`.

### identity

Stores the local user's identity metadata. One row per identity (multi-account
support).

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment row ID |
| public_key | TEXT UNIQUE | Ed25519 public key (hex) |
| display_name | TEXT | User-chosen display name |
| created_at | INTEGER | Unix timestamp of creation |
| dht_record_key | TEXT | DHT profile record key |
| dht_owner_keypair | TEXT | Keypair for DHT profile write access |
| friend_list_dht_key | TEXT | DHT friend list record key |
| friend_list_owner_keypair | TEXT | Keypair for friend list write access |
| avatar_webp | BLOB | Avatar image (WebP format) |
| account_dht_key | TEXT | Account recovery DHT record key |
| account_owner_keypair | TEXT | Keypair for account record write access |
| mailbox_dht_key | TEXT | Mailbox DHT record key (route blob inbox) |

### friends

Stores the friend list with per-identity scoping.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity this friend belongs to |
| public_key | TEXT | Friend's Ed25519 public key |
| display_name | TEXT | Friend's display name |
| nickname | TEXT | Local nickname override |
| group_id | INTEGER FK | Friend group assignment |
| added_at | INTEGER | Unix timestamp when added |
| dht_record_key | TEXT | Friend's DHT profile key (for presence watching) |
| last_seen_at | INTEGER | Last online timestamp |
| avatar_webp | BLOB | Cached avatar |
| local_conversation_key | TEXT | Our conversation DHT record key |
| local_conversation_keypair | TEXT | Keypair for our conversation record |
| remote_conversation_key | TEXT | Friend's conversation DHT record key |
| mailbox_dht_key | TEXT | Friend's mailbox DHT key (route blob fallback) |

Primary key: `(owner_key, public_key)`

Index: `idx_friends_group_id` on `(owner_key, group_id)`

### friend_groups

Collapsible groups in the buddy list (e.g., "Gaming", "Work").

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment |
| owner_key | TEXT FK | Identity that owns this group |
| name | TEXT | Group display name |
| sort_order | INTEGER | Display ordering |

Unique constraint: `(owner_key, name)`

### messages

All chat messages — both 1:1 DMs and community channel messages.

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment |
| owner_key | TEXT FK | Identity this message belongs to |
| conversation_id | TEXT | Peer public key (DM) or channel ID |
| conversation_type | TEXT | `dm` or `channel` |
| sender_key | TEXT | Sender's public key |
| body | TEXT | Message body (plaintext after decryption) |
| timestamp | INTEGER | Unix timestamp |
| is_read | INTEGER | 0 = unread, 1 = read |
| reply_to_id | INTEGER FK | Referenced message (nullable) |
| attachment_json | TEXT | Attachment metadata (JSON, nullable) |
| mek_generation | INTEGER | MEK generation for channel message decryption |

Indexes:
- `idx_messages_conversation` on `(owner_key, conversation_id, timestamp)`
- `idx_messages_unread` on `(owner_key, conversation_id, is_read)` where `is_read = 0`
- `idx_messages_dedup` unique on `(owner_key, conversation_id, conversation_type, sender_key, timestamp)` (deduplication)

### communities

Joined communities with per-identity scoping.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| id | TEXT | Community ID |
| name | TEXT | Community name |
| description | TEXT | Community description |
| icon_hash | TEXT | Icon content hash |
| my_role | TEXT | Our role (default: `member`) |
| my_role_ids | TEXT | Comma-separated role IDs (new multi-role system) |
| joined_at | INTEGER | Unix timestamp |
| last_synced | INTEGER | Last DHT sync timestamp |
| dht_record_key | TEXT | Community DHT record key |
| dht_owner_keypair | TEXT | Keypair for DHT record write access |
| my_pseudonym_key | TEXT | Our unlinkable pseudonym key for this community |
| mek_generation | INTEGER | Current MEK generation (default: 0) |
| server_route_blob | BLOB | Community server's Veilid route blob |
| is_hosted | INTEGER | Whether we are hosting this community's server |

Primary key: `(owner_key, id)`

### channels

Channels within communities.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| id | TEXT | Channel ID |
| community_id | TEXT FK | Parent community |
| name | TEXT | Channel name |
| channel_type | TEXT | `text` or `voice` |
| sort_order | INTEGER | Display ordering |

Primary key: `(owner_key, id)`

Index: `idx_channels_community_id` on `(owner_key, community_id)`

### community_members

Membership records for communities.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| community_id | TEXT FK | Community ID |
| pseudonym_key | TEXT | Member's pseudonym public key (unlinkable per community) |
| display_name | TEXT | Member's display name |
| role_ids | TEXT | Comma-separated role IDs |
| timeout_until | INTEGER | Timeout expiry timestamp (nullable) |
| joined_at | INTEGER | Unix timestamp |

Primary key: `(owner_key, community_id, pseudonym_key)`

Index: `idx_community_members_community` on `(owner_key, community_id, pseudonym_key)`

### community_roles

Role definitions for communities.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| community_id | TEXT FK | Community ID |
| role_id | INTEGER | Role ID |
| name | TEXT | Role name |
| color | INTEGER | RGB packed color |
| permissions | INTEGER | Permission bitmask (u64) |
| position | INTEGER | Display ordering |
| hoist | INTEGER | Whether to show role separately in member list |
| mentionable | INTEGER | Whether role can be @mentioned |

### channel_overwrites

Per-channel permission overrides for roles or members.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| community_id | TEXT FK | Community ID |
| channel_id | TEXT FK | Channel ID |
| target_type | TEXT | `role` or `member` |
| target_id | TEXT | Role ID or member pseudonym key |
| allow | INTEGER | Permission bitmask to allow |
| deny | INTEGER | Permission bitmask to deny |

### trusted_identities

TOFU identity key tracking for key continuity.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| public_key | TEXT | Peer's public key |
| identity_key | BLOB | Peer's identity key bytes |
| verified | INTEGER | 0 = TOFU, 1 = verified out-of-band |
| first_seen | INTEGER | Unix timestamp of first contact |

### signal_sessions

Persisted Signal Protocol session state.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| recipient_key | TEXT | Peer's public key |
| session_data | BLOB | Serialized session state |
| updated_at | INTEGER | Last modification timestamp |

### prekeys

Signal Protocol prekey storage.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| id | INTEGER | PreKey ID |
| key_data | BLOB | Serialized key data |
| is_signed | INTEGER | 0 = one-time, 1 = signed |
| created_at | INTEGER | Unix timestamp |

### pending_messages

Offline message queue for retry delivery.

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-increment |
| owner_key | TEXT FK | Identity |
| recipient_key | TEXT | Intended recipient |
| body | TEXT | Serialized message body |
| created_at | INTEGER | Unix timestamp |
| retry_count | INTEGER | Number of delivery attempts (max 20) |

Index: `idx_pending_recipient` on `(owner_key, recipient_key)`

### pending_friend_requests

Incoming friend requests awaiting user action.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| public_key | TEXT | Requester's public key |
| display_name | TEXT | Requester's display name |
| message | TEXT | Optional request message |
| received_at | INTEGER | Unix timestamp |
| profile_dht_key | TEXT | Requester's DHT profile key |
| route_blob | BLOB | Requester's Veilid route blob |
| mailbox_dht_key | TEXT | Requester's mailbox DHT key |
| prekey_bundle | BLOB | Requester's Signal PreKeyBundle |

### blocked_users

Users blocked from sending messages.

| Column | Type | Description |
|--------|------|-------------|
| owner_key | TEXT FK | Identity |
| public_key | TEXT | Blocked user's public key |
| blocked_at | INTEGER | Unix timestamp |

## Schema Versioning

The schema version is tracked by a `SCHEMA_VERSION` constant in `db.rs`. When
the constant is incremented and the application starts, the database detects a
mismatch and drops all tables, recreating them from `001_init.sql`.

Because SQLite, Stronghold, and Veilid DHT store interrelated state (friend
keys, DHT record keypairs, Signal sessions), a schema reset also triggers:

1. Deletion of all `.stronghold` files in the config directory
2. Removal of the `veilid/` local storage directory

This ensures the three stores remain synchronized. Migration files are not used
because the schema is not yet stable for production.

## Database Access Pattern

The connection pool is `Arc<std::sync::Mutex<Connection>>` (standard library
Mutex, not `parking_lot`), used with `spawn_blocking` to avoid blocking the
async runtime. The `rusqlite` crate (version 0.37) is used instead of `sqlx` to
match `veilid-core`'s dependency on the same version and avoid `libsqlite3-sys`
build conflicts.

## Stronghold Vault

Each identity has a dedicated `.stronghold` file in the application config
directory. The vault is encrypted with a key derived from the user's passphrase
via Argon2id.

### Vault Namespaces

| Vault | Key | Purpose |
|-------|-----|---------|
| `identity` | `ed25519_private` | Ed25519 signing private key |
| `identity` | `x25519_private` | X25519 Diffie-Hellman private key |
| `signal` | `identity_keypair` | Signal Protocol identity keypair |
| `signal` | `signed_prekey` | Current signed prekey |
| `signal` | `prekey_batch` | Batch of one-time prekeys |
| `veilid` | `protected_store_key` | Veilid protected store encryption key |
| `communities` | `mek_{community_id}` | Per-community Media Encryption Key |

Stronghold is accessed through `iota_stronghold` directly (not via the Tauri
Stronghold plugin) to allow per-identity snapshot files and configurable Argon2
parameters for debug builds.

## DHT Record Layout

### User Profile Record (DFLT, 8 subkeys)

| Subkey | Content | Format |
|--------|---------|--------|
| 0 | Display name | UTF-8 |
| 1 | Status message | UTF-8 |
| 2 | Status enum | UTF-8 (`online`, `away`, `busy`, `offline`) |
| 3 | Avatar | WebP bytes |
| 4 | Game info | Cap'n Proto `GameInfo` |
| 5 | PreKeyBundle | Cap'n Proto `PreKeyBundle` |
| 6 | Route blob | Raw bytes (Veilid private route) |
| 7 | Metadata | Reserved |

### Friend List Record

A DHT record containing serialized friend list entries. Each entry holds a
public key and display name. Used by peers to discover mutual friends and
verify friend relationships.

### Mailbox Record (DFLT, 1 subkey)

Each user publishes a mailbox DHT record created with their identity keypair
(deterministic key). Contains only the user's current Veilid route blob, providing
a fallback way for peers to find a route when the profile record's subkey 6 is stale.

| Subkey | Content |
|--------|---------|
| 0 | Route blob (raw bytes) |

### Community Records (SMPL, multi-writer, 7 subkeys)

Communities use SMPL (multi-writer) DHT records to allow multiple admins to
update community metadata, channel lists, and member rosters.

| Subkey | Content |
|--------|---------|
| 0 | Metadata (name, description, icon, owner key) |
| 1 | Channels list (JSON) |
| 2 | Members list (JSON) |
| 3 | Roles (JSON) |
| 4 | Invites |
| 5 | MEK (encrypted, per-member bundles) |
| 6 | Server route blob |

### Account Record (DFLT, encrypted)

Private account record encrypted with `DhtRecordKey::derive_account_key()` from
the user's Ed25519 secret. Contains three child `DHTShortArray` references for
contacts, chats, and invitations.

### Conversation Record (DFLT, encrypted)

Per-friend-pair conversation record encrypted with `DhtRecordKey::derive_conversation_key()`
from X25519 DH shared secret. Contains a child `DHTLog` for message history.

## Cap'n Proto Serialization

All structured data exchanged over the network is serialized with Cap'n Proto
for zero-copy deserialization and schema evolution. Schema files live in
`schemas/` and are compiled at build time via `capnpc` in each crate's
`build.rs`.

| Schema | Structs |
|--------|---------|
| `message.capnp` | `MessageEnvelope`, `DirectMessage`, `ChannelMessage` |
| `identity.capnp` | `IdentityRecord`, `PreKeyBundle` |
| `presence.capnp` | `PresenceRecord`, `GameInfo` |
| `friend.capnp` | `FriendRequest`, `FriendResponse`, `FriendListEntry` |
| `community.capnp` | `CommunityRecord`, `ChannelRecord`, `MemberRecord`, `RoleRecord` |
| `voice.capnp` | `VoiceSignaling`, `VoicePacket` |
| `conversation.capnp` | `ConversationRecord` |
| `account.capnp` | `AccountRecord` |

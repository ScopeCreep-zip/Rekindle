# Data Storage and Serialization

Rekindle uses three persistent backends, each serving a distinct
purpose. SQLite stores local state and message history. The
**Vault** (`rekindle-vault`, SQLCipher double-encryption) replaces
the previous `iota_stronghold` dependency and holds private keys,
and MEKs. Signal keying material (ratchet sessions, prekeys) lives in the identity's SQLCipher database. Veilid DHT provides distributed
storage for profile data, presence, and community state.

## Storage Backend Summary

| Backend | Scope | Contents |
|---------|-------|----------|
| SQLite | Local device | Identity, friends, messages, communities, ratchet sessions and prekeys, DMs, sync state, audit chain, analytics, pending messages |
| Vault (`rekindle-vault`) | Local device | Ed25519 / X25519 private keys, MEKs, audit MAC key, slot keypairs |
| Veilid DHT | Distributed | Profile, presence, mailbox, friend list, community governance / registry / channels, account record, personal sync record |
| Filesystem | Local device | Lost Cargo per-identity chunk cache (`<data>/state/{pk}.files/<scope name>/...`, keyed-hash directory names) |

The `event_journal` field on `AppState` is **in-memory only** — it
does not touch SQLite or the vault — but it is the fourth piece of
persistent state for the event-resume flow within a single process
lifetime. See [`event-dispatch.md`](event-dispatch.md).

## Data Root

Every host keeps its files under one `DataRoot`
(`rekindle_utils::paths`, re-exported as `rekindle_db::paths`), named
after the bundle identifier `com.rekindle.app`. Its directories are
Tauri 2's app directories, computed with the same `dirs` release Tauri
uses, so the desktop, `rekindled`, the CLI and the TUI all resolve the
same folders:

| Field | Location | Holds |
|---|---|---|
| `data` | `dirs::data_dir()/com.rekindle.app` | the node lock (`node.lock`); `file_cache/` only for community avatars until plan E3.1 |
| `state` | `data/state` | every identity's files: `{pk}.vault` (+ `.hdr`), `{pk}.db`, `{pk}.files/` (the file cache); the registry `identities.json`; `layout-version`; Veilid storage (`veilid/`) |
| `config` | `dirs::config_dir()/com.rekindle.app` | `config.toml` (user layer) |
| `logs` | `~/Library/Logs/com.rekindle.app` (macOS), `dirs::data_local_dir()/com.rekindle.app/logs` elsewhere | `rekindled.log`, `rekindle.log`, `rekindle-tui.log` (the desktop logs to stdout) |

On macOS `data` and `config` are the same folder. `create_dirs` makes
every directory owner-only (`0700`). `rekindle_db::lock::NodeLock` holds
`data/node.lock` for the life of the process, so the desktop and
`rekindled` refuse to run on the same root at once.

### Identities (plan D1)

Each identity has its own files under `state/`, and both hosts reach them
through one implementation, `rekindle_node::host::identity`
(`list`, `create`, `unlock`, `lock`, `delete`, `remove`, `rename`,
`rotate`, `export`, `import`, `verify_passphrase`):

- `{pk}.vault` and its 64-byte header `{pk}.vault.hdr` (Argon2id
  parameters and salt): the identity's secrets, sealed by its passphrase
  (`rekindle-vault`).
- `{pk}.db`: its SQLCipher database, keyed by the vault-derived
  `AppDbKey`; it opens only after an unlock. Every connection runs WAL
  with `foreign_keys`, `secure_delete`, `synchronous=FULL`, `fullfsync`
  and `checkpoint_fullfsync` on (`rekindle_db::open`): on macOS a plain
  `fsync` does not reach the disk platter, and a ratchet step committed
  before its message is sent must survive a power loss (SQLite applies
  the two full-fsync flags on macOS only).
- `identities.json`: the registry the login picker and `IdentityList` read
  while locked (public key, display name, creation time; no secret).

`layout-version` holds the `SCHEMA_VERSION` the root was laid out for.
Both hosts run `rekindle_db::reset::apply` at startup, under the node
lock and before any store opens: a root laid out for another version is
reset. A wrong passphrase counts on the host's one per-identity limiter;
the fifth consecutive failure locks the identity out, escalating.

The daemon's session (`rekindle_transport::Session`) is a projection of
the unlocked identity's database and vault (`rekindle_node::host::session`);
`session.json` and the OS keyring are gone.

## SQLite Schema

Each identity's database is `DataRoot::identity_db(pk)` (`state/{pk}.db`;
see [Data Root](#data-root)). All tables are defined in
`crates/rekindle-db/schema/001_init.sql`; `SCHEMA_VERSION` in
`crates/rekindle-db/src/open.rs` is bumped whenever that file changes.
The SQL for the tables more than one host uses is in
`rekindle_db::repo` (see [Repositories](#repositories)).

`001_init.sql` defines 47 tables. The remainder of this section
groups them by concern.

### Identity and Friends

- **`identity`** — local user identity. One row per identity
  (multi-account). Includes Ed25519 public key, display name, DHT
  record keys for profile / friend list / account / mailbox /
  personal sync, optional avatar BLOB, and a stable per-device
  `device_id` (16 random bytes, 32 lowercase hex characters, `CHECK`ed),
  minted once by the own-records stage; the prekey device list and the
  cross-device DeviceList name it and nothing replaces it. The owned
  records' owner keypairs are in the vault, each entry holding the
  record key with its keypair (`rekindle_vault::typed::owner_keys`): the
  entry is written first, and the unlock repairs the column from an entry
  that names another record (a crash between the two writes).
- **`signal_identities`** — peers' identity keys (TOFU) with the
  `verified` flag and `verified_at` for out-of-band safety-number
  confirmation; `changed_from_key` / `changed_at` hold an unacknowledged
  change of a verified key (the `VerifiedKeyChanged` escalation).
- **`friend_groups`** — collapsible buddy list groupings.
- **`friends`** — friend list with per-identity scoping. Tracks
  display name, nickname override, group, presence DHT record key,
  last-seen, cached avatar, conversation DHT keys, mailbox key, and
  `friendship_state` (`accepted` / `pendingOut` / `removing`).
- **`blocked_users`** — blocked public keys per identity.

### Messaging

- **`messages`** — chat messages for both 1:1 DMs and community
  channels. Indexed by `(owner_key, conversation_id, timestamp)`.
  Carries `message_id` (channel messages), `mek_generation`,
  `lamport_ts`, `automod_blurred` flag, `forwarded_from_author`, and a
  `flags` bitmask. A row with a client id (`message_id`, every DM) is
  deduplicated by `(sender_key, message_id)`, so two messages in one
  millisecond are both kept; a row without one by `(conversation_type,
  sender_key, timestamp)`.
- **`pending_messages`** — offline retry queue (max 20 retries).
- **`pending_friend_requests`** — incoming friend requests awaiting
  user action; carries the requester's profile / route / mailbox info
  and optional `invite_id` (no prekeys: the accepter reads the
  requester's prekey record).
- **`outgoing_invites`** — sent invites tracked by `invite_id` with
  status (`pending` / `responded` / `accepted` / `rejected` /
  `cancelled` / `expired`).
- **`message_delivery`** — per-message per-recipient delivery
  tracking for community channel messages.

### Communities

- **`communities`** — joined communities. Tracks DHT record keys
  (`manifest_key`, `member_registry_key`, `governance_key`), our
  pseudonym, current MEK generation, plate-gate `my_segment_index`,
  `my_subkey_index`, and `lamport_clock`.
- **`channels`** — channels within communities. Types: `text`,
  `voice`, `announcement`, `forum`, `stage`, `directory`, `media`,
  `events`, `dm`. Includes `topic`, `slowmode_seconds`, `nsfw`,
  per-channel `message_record_key`, `mek_generation`, `log_key`, and
  `my_sequence` (our send counter for gap detection).
- **`community_categories`** — channel categories.
- **`community_members`** — pseudonym registry per community. Carries
  roles, timeout, segment index, onboarding flag, and per-community
  profile fields (`bio`, `pronouns`, `theme_color`, `badges`,
  `avatar_ref`, `banner_ref`).
- **`community_member_leaves`** — append-only departure log for
  analytics (live `community_members` rows are deleted on leave).
- **`community_roles`** — role definitions. Includes `permissions`
  bitmask, `position`, `hoist`, `mentionable`, `self_assignable`, and
  optional `exclusion_group` (architecture §19.4).
- **`channel_overwrites`** — per-channel allow / deny bitmask
  overrides for a role or member.
- **`channel_slowmode_state`** — per-channel slowmode timestamp
  state.
- **`channel_read_state`** — per-channel last-read Lamport.
- **`channel_pins`** — pinned messages per channel.
- **`community_threads`** + **`thread_messages`** — forum-style
  sub-conversations.
- **`community_events`**, **`event_rsvps`**, **`community_event_rsvps`**
  — scheduled events.
- **`community_invites`** — locally-cached invite codes for invites
  we created.
- **`game_servers`** — community game-server favourites.
- **`peer_reliability`** — Mutual Aid topology metrics (architecture
  §14.5): per-peer per-community success / failure counts.

### Governance and Audit

- **`governance_entries_cache`** — hydrated cache of merged
  `GovernanceEntry` rows so the CRDT merge does not need to re-fetch
  from the DHT on every command.
- **`audit_entries`** — tamper-evident audit log. Each row carries
  `prev_mac` + `mac` per [`audit-chain.md`](audit-chain.md). The MAC
  key lives in the vault; loss of the vault loses the chain.

### Calls

- **`missed_calls`** — missed-call log surfaced in the buddy list.
- **`voice_session_events`** — append-only join / leave log for
  computing peak-concurrent-voice metrics per channel.

### Signal Protocol

- **`ratchet_sessions`** — one `rekindle_crypto::SessionRecord` (the spec
  Double Ratchet state) per peer identity and device, stepped and stored in
  the same transaction as whatever the message changed, before any send
  (`rekindle_db::ratchet`).
- **`signal_prekeys`** — our prekeys by `kind` (`spk` signed, `pq_lr`
  ML-KEM last resort, `opk` / `pq_opk` one-time), each with the record
  subkey publishing it (`published_subkey`), `consumed_at` and
  `retired_at`; `rekindle_db::prekeys`.
- **`prekey_slots_used`** — the slots of a peer device's prekey record we
  initiated from (B2's pick skips them); swept once the record no longer
  lists the id, or after 90 days.
- **`prekey_rejections`** — the `PreKeyRejected` notices we sent (B5): one
  per initiator base key, at most 10 per sender per 3 h; kept 30 days.
- **`prekey_lr_inits`** — the `(lr_id, spk_id, base_key)` of every init
  that opened a session on the last-resort key (B8's record of LR use,
  B9's replay set); kept while that last-resort key is held.
- **`unprocessed`** — decrypted inbound 1:1 messages not yet applied,
  written in the ratchet transaction, deleted by the apply and replayed at
  unlock (D24); keyed by ratchet header `(header_dh, header_n)`. Kept at
  most `UNPROCESSED_MAX_AGE` (1.5 × `DELIVERY_HORIZON`) or 10 attempts; a
  row that does not read back is quarantined (`quarantined_at`, skipped
  by every replay) rather than failing the replay of the others.

Prekey lifecycle (plan E2.4, PQXDH §3.2–§3.3): the rows back this
device's prekey record (see [Prekey Record](#prekey-record-dflt101)). Every
id is a random non-zero u31, unique per kind. The "prekey maintenance"
task (hourly, both hosts) refills a consumed slot, fills every empty slot
once fewer than 10 are fresh, replaces a slot older than 90 days, and
rotates the head (signed prekey + last-resort key) every 2 days; the
`crypto.prekeys` status check warns when fewer than 10 slots are fresh or
the head is older than 14 days. A replaced key is marked `retired_at` and
still opens an init in flight; the archive sweep deletes retired and
consumed rows 30 days on. A one-time key is consumed only after the first
message under it decrypts (`rekindle_db::ratchet::accept_init`, B8): in one
transaction its secret is erased, an LR use is recorded, the session is
stored and the slot is refilled; the refilled slot is written to the
record after the commit, and the unlock reconciliation re-publishes any
row the record does not match.

### Notifications & Settings

- **`notification_preferences`** — per-channel / per-community level
  (0 = All, 1 = Mentions, 2 = Mute) plus optional `sound_ref`
  (community soundboard expression ID). Falls through channel →
  community → app-global.
- **`app_settings`** — global app settings: quiet hours window /
  timezone offset and the global Do Not Disturb toggle.

### Strand Relay and Push Relay

- **`strand_relay_offers`** — for each friend who volunteered to
  relay for us, their dedicated relay route blob and timestamp.
- **`strand_relay_volunteered`** — outbound: friends we've volunteered
  to relay for.
- **`push_relay_registrations`** — relay pseudonym, device push
  token, platform (`fcm` / `apns` / `self`), JSON list of record
  keys.

### Cross-Device Sync (Architecture §28.4)

- **`paired_devices`** — local cache mirror of the personal DFLT
  record's device list subkey.
- **`pending_pairings`** — short-lived rows holding the active
  pairing code and salt.

### Full-Text Search (Architecture §23)

Two FTS5 virtual tables with `unicode61 remove_diacritics 2`
tokenizer (no `porter` stemming — Rekindle is multilingual):

- `messages_fts` — content table `messages`, content_rowid `id`
- `thread_messages_fts` — content table `thread_messages`, content_rowid `id`

Each has `AFTER INSERT/UPDATE/DELETE` triggers keeping the index
in lock-step.

### Performance Indexes

- `idx_friends_group_id` on `friends(owner_key, group_id)`
- `idx_messages_conversation` on `messages(owner_key, conversation_id, timestamp)`
- `idx_messages_unread` on `messages(owner_key, conversation_id, is_read)` where `is_read = 0`
- `idx_messages_dedup` UNIQUE on `messages(owner_key, conversation_id, conversation_type, sender_key, timestamp)` where `message_id IS NULL`
- `idx_messages_message_id` UNIQUE on `messages(owner_key, conversation_id, sender_key, message_id)` where `message_id IS NOT NULL`
- `idx_channels_community_id` on `channels(owner_key, community_id)`
- `idx_community_members_community` on `community_members(owner_key, community_id, pseudonym_key)`
- `idx_pending_recipient` on `pending_messages(owner_key, recipient_key)`
- `idx_thread_messages_thread` on `thread_messages(owner_key, community_id, thread_id, timestamp)`
- `idx_member_leaves_recent` on `community_member_leaves(owner_key, community_id, left_at)`
- `idx_voice_session_events_channel` on `voice_session_events(owner_key, community_id, channel_id, occurred_at)`

## Schema Versioning

The schema version is tracked by the `SCHEMA_VERSION` constant in
`crates/rekindle-db/src/open.rs`. When the constant is incremented and
the application starts, `rekindle_db::open` detects the mismatch, drops
all tables and recreates them from `001_init.sql`, reporting
`schema_reset` so the host wipes the storage below.

Because SQLite, the vault, and Veilid DHT store interrelated state
(friend keys, DHT record keypairs), a schema reset
also triggers:

1. Removal of the vault store and its `.hdr` header.
2. Removal of the `veilid/` local storage directory.
3. Removal of the Lost Cargo file cache root.

This keeps the three stores synchronised. Migration files are not
used because the schema is not yet stable for production —
[`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md)
covers the pre-ship posture and why hard breaks are acceptable today.

## Database Access Pattern

The handle is `rekindle_db::Db`: a `rekindle_asql::Connection` (the
vendored tokio-rusqlite) running `rusqlite` on a dedicated background
thread, plus a liveness token plan C6 checks before closing it. In the
desktop all access goes through
`db_helpers::{db_call, db_fire}`. Read-only state
lookups go through `state_helpers`.

`rusqlite` 0.39 is used (not `sqlx`) to match `veilid-core` 0.5.7's
`keyvaluedb-sqlite` → `async-sqlite` → `rusqlite` 0.39 chain; one
`libsqlite3-sys` (0.37, bundled SQLCipher) serves the whole process.

### Repositories

`rekindle_db::repo` holds the queries for the tables more than one host
uses. Each function takes `&rusqlite::Connection` and runs inside a
`Db::call` closure, so several share one transaction:

| Module | Table |
|---|---|
| `identity` | `identity` |
| `friends` | `friends` (also read by `SqliteFriendStore`) |
| `pending_requests` | `pending_friend_requests` |
| `communities` | `communities` |
| `members` | `community_members` (owner-scoped; `members::roles` for role sets) |
| `governance_cache` | `governance_entries_cache` |
| `audit` | `audit_entries` |

Repository tests run against the real schema. `cargo xtask check-sqlite`
rejects SQL and SQLite driver dependencies outside `rekindle-db`,
`rekindle-vault` and `rekindle-asql`, apart from a shrinking list of
crates the plan's E phase empties.

## Vault (`rekindle-vault`)

The vault is **double-encrypted**:

`master = Argon2id(passphrase, salt)` (64 bytes), and three keys are
`blake3::derive_key` of it under registered labels
(`rekindle_types::domains`):

1. **SQLCipher** (AES-256-CBC, page-level) encrypts the entire
   vault file, keyed under `rekindle-vault-sqlcipher-v1`.
2. **Per-entry AES-256-GCM** seals every `entries.ciphertext` row
   independently (AAD `namespace ‖ 0x00 ‖ key`), keyed under
   `rekindle-vault-entry-v1`.
3. The **app-db key** (`AppDbKey`, `rekindle-vault-app-db-v1`) keys the
   identity's own SQLCipher database (`state/{pk}.db`), so one passphrase
   unlocks both.

The Argon2id parameters and the 32-byte salt live in the 64-byte header
`state/{pk}.vault.hdr`, plaintext (neither is secret) and readable
before decryption; `VaultStore::create` writes it and `open` never
creates one. The vault itself is `state/{pk}.vault` (per identity, so
multi-account works without collisions).

The identity database, not the vault, holds the 1:1 ratchet sessions and
the prekey secrets (plan D3: single-layer under SQLCipher, ruling R2).
It is never exported: an identity export carries the registry entry, the
vault and its header only, and the import creates a fresh database
(sessions are re-established, prekeys re-minted). Secrets erased from it
are not left in its WAL: `secure_delete` zeroes freed pages,
`journal_size_limit=0` truncates the WAL at each reset, and a prekey
consumption (`ratchet::accept_init`) and the archive sweep
(`prekeys::sweep_archive`) run `wal_checkpoint(TRUNCATE)` after they
commit (DR §8.1).

The on-disk schema is a single table:

```sql
CREATE TABLE entries (
  namespace TEXT NOT NULL,
  key       TEXT NOT NULL,
  nonce     BLOB NOT NULL,
  ciphertext BLOB NOT NULL,
  …
);
```

`rekindle_vault::typed` holds one module of typed helpers per kind of
secret, so every host stores them the same way (the desktop's
`src-tauri/src/keystore/` only opens the per-identity store and
re-exports them):

| Module | Namespace / key pattern | Purpose |
|---|---|---|
| `typed::community_keys` | `communities` / `slot_keypair_{community_id}`, `slot_seed_{community_id}`, `registry_keypair_{community_id}` | Per-community slot / registry keypairs and slot seed |
| `typed::mek` | `communities` / `mek_{community}`, `community_mek_{community}_{generation}`, `community_mek_generations_{community}`, `mek_{community}_{channel}`, `mek_{community}_{channel}_{generation}`, `mek_generations_{community}_{channel}` | Every MEK generation per scope, the scope's latest key, and the generations index that leaving a community erases |
| `typed::audit` | `audit` / `mac_key`, `tail` | BLAKE3-keyed MAC key + tail anchor for the audit chain |

The identity Ed25519 / X25519 secret bytes live in
`AppState.identity_secret: Mutex<Option<[u8; 32]>>` while the
session is unlocked; the at-rest copy is namespaced under
`identity` in the same vault.

See [`decisions/0006-vault-replaces-stronghold.md`](../decisions/0006-vault-replaces-stronghold.md)
for the migration rationale and the threat-model implications.

## DHT Record Layout

### User Profile Record (DFLT, 9 subkeys)

| Subkey | Content | Format |
|--------|---------|--------|
| 0 | Display name | UTF-8 |
| 1 | Status message | UTF-8 |
| 2 | Status enum | UTF-8 (`online`, `away`, `busy`, `offline`, `invisible`) |
| 3 | Avatar | WebP bytes |
| 4 | Game info | Cap'n Proto `GameStatus` |
| 5 | Prekey records | Cap'n Proto `PrekeyRecordList` (each device's id and prekey record key; one entry until E5.3) |
| 6 | Route blob | Raw bytes (Veilid private route) |
| 7 | Metadata | Reserved (extensible header) |
| 8 | Strand Relay pool | JSON list of opaque relay route blobs, padded with dummies |

### Prekey Record (DFLT(101))

One record per device, owned by a keypair of its own (vault
`OwnerKey::Prekeys`, key in `identity.prekey_record_key`) and named from
profile subkey 5. It is created or reopened by the unlock's own-records
stage, reconciled and topped up before the profile names it, and survives
an identity rotation: `rotate_identity` publishes the new identity's full
set into the same record.

| Subkey | Content | Format |
|--------|---------|--------|
| 0 | Head: signed prekey and ML-KEM-768 last-resort key, their ids, signatures and `issuedAt` | Cap'n Proto `PrekeyRecordHead` |
| 1..=100 | One slot each: a one-time X25519 prekey and a one-time ML-KEM-768 key, ids and signatures | Cap'n Proto `OpkSlot` |

Each subkey value is capped at 10,381 B (`min(32,768, 1,048,576 / 101)`,
pinned by `dht_layout::prekeys::MAX_SUBKEY_VALUE_LEN`). Every signature is
the identity's Ed25519 over a domain label, the id, the key and, for the
head's two keys, the head's `issued_at`:
`PREKEY_SPK_SIG ‖ u32be(spk_id) ‖ i64be(issued_at) ‖ EncodeEC(spk)`,
`PREKEY_PQ_LR_SIG ‖ u32be(id) ‖ i64be(issued_at) ‖ EncodeKEM(pqpk)`,
`PREKEY_PQ_OT_SIG ‖ u32be(id) ‖ EncodeKEM(pqpk)` and
`PREKEY_OPK_SIG ‖ u32be(opk_id) ‖ EncodeEC(opk)` (labels in
`rekindle-types/src/domains.rs`; `rekindle-crypto::signal::pqxdh::verify`).

### Friend List Record

A DHT record containing serialized friend list entries (Cap'n Proto
`FriendEntry` array). Used by peers to discover mutual friends and
verify friend relationships.

### Mailbox Record (DFLT, 1 subkey)

Each user publishes a mailbox DHT record created with their identity
keypair (deterministic key). Contains only the user's current Veilid
private route blob, providing a fallback when a friend's profile
subkey 6 is stale.

### Account Record (DFLT, encrypted)

Private account record encrypted with
`DhtRecordKey::derive_account_key()` from the user's Ed25519 secret.
Contains three child `DHTShortArray` references for contacts, chats,
and invitations.

### Personal Sync Record (DFLT, encrypted, architecture §28.4)

Lazy-created on first opt-in to multi-device. Subkeys hold the device
list, read state per channel, sync preferences, paired device public
keys, and sync manifest. Reconciled by the cross-device sync service.

### Community Records (SMPL `o_cnt:0`, 255 subkeys — universal v2.0 schema)

All community records share the universal SMPL schema. The
`o_cnt: 0` property means the creation keypair is discarded after
genesis — the record has no privileged owner.

- **Governance record** — every member writes their
  `GovernanceEntry` history to their assigned subkey. Reader merges
  all subkeys via `rekindle-governance::merge::merge()`.
- **Member registry record** — a presence directory, not a ledger.
  Members claim a free subkey on join and overwrite it each
  heartbeat; it never grows. Membership is a predicate over these
  rows (validly signed, not banned, not departed), computed by
  `rekindle_governance_runtime::roster`. Nothing community-wide
  lives here: the member index, MEK vault and moderation queue were
  v1.0 owner subkeys that `o_cnt: 0` credentials nobody to write.
- **Channel records** — one SMPL record per `(channel, segment)` for
  message persistence, each member writing to their own slot subkey.
  Same 255-subkey layout. The segment-0 record key is
  `ChannelCreated.record_key` in merged governance; further segments
  are announced with `ChannelSegmentLinked`, so discovery needs no
  registration step.

When a community grows beyond 255 members, **Plate Gates** add
fractal SMPL segments. `community_members.segment_index` and
`my_segment_index` track which segment hosts each member's slot.
See [`communities-governance.md`](communities-governance.md).

### DM Record (SMPL `o_cnt:0`, 2 or N subkeys)

DMs are SMPL records with `o_cnt:0`, exactly two member subkeys for
2-party DMs (or N for group DMs). The MEK is derived
deterministically via X25519 ECDH between identity keys (no
key-exchange round-trip). Group DMs wrap the MEK per recipient.

## Lost Cargo File Cache (Architecture §28.9)

Per-identity filesystem cache for chunked attachments managed by
`rekindle-files`. Path layout:

```
<data>/state/{pk}.files/<scope name>/<attachment name>/<chunk_index>.bin
```

Every directory name is a 32-hex BLAKE3-keyed hash (scope name =
`keyed_name(k_names, "community:" ‖ id)`, also `dm:` and `avatars:`;
attachment name keyed from `"attachment:" ‖ scope name ‖ ":" ‖ id`;
`k_names` = `derive_file_cache_name_key(master_secret)`, HKDF-SHA256,
salt `rekindle-file-cache-names-v1`). Names are stable across restarts
and unlinkable without the secret. Identity delete removes the tree.
Eviction runs synchronously after every `cache.insert()` until
`total_bytes ≤ budget`; pinned attachments are skipped. Pinned-
attachment IDs come from the merged
`governance_state.pinned_attachments` set.

## Cap'n Proto Serialization

Network-facing structured data is serialised with Cap'n Proto for
zero-copy deserialization and schema evolution. Schemas live in
`schemas/` and are compiled at build time via `capnpc` in
`rekindle-protocol`'s `build.rs`. The generated Rust modules are
included at the protocol crate root via
`pub mod foo_capnp { include!(...) }` so generated `crate::<schema>_capnp`
paths resolve.

| Schema | Top-level structs |
|--------|-------------------|
| `direct.capnp` | `DmFrame`, `DmContent`, `SealedContent`, the 1:1 kind bodies |
| `identity.capnp` | `PrekeyRecordHead`, `OpkSlot`, `PrekeyRecordList` |
| `presence.capnp` | `PresenceUpdate`, `GameStatus` |
| `friend.capnp` | `FriendRequest`, `FriendList`, `FriendEntry` |
| `community.capnp` | `Community`, `Channel`, `Role`, `PermissionOverwrite` |
| `voice.capnp` | `VoiceSignaling` |
| `account.capnp` | `AccountHeader`, `ContactEntry`, `ChatEntry` |

`postcard` / `bincode` is used for the v2.0 community gossip
envelope — those are internal binary formats not
intended for cross-language use.

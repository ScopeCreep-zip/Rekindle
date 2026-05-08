# Cross-Device Sync

Cross-device sync lets one identity run on multiple devices (laptop +
desktop, desktop + phone) with consistent friend lists, read state,
preferences, and a unified device list — without any server-side
session storage.

The data lives in a personal **DFLT** DHT record owned by the identity's
master secret. Every paired device watches the record and merges
updates locally. There is no central authority, no relay server, no
session store; the record itself is the rendezvous.

## The personal sync record

A single DFLT (single-owner, multi-subkey) record per identity, with
**4 well-known subkeys** mapped to specific concerns:

```
DFLT { o_cnt: 4 }
├── Subkey 0: Sync manifest
│       Encrypted directory of communities, friends, DM threads, paired
│       devices — the index a fresh device needs to bootstrap.
│
├── Subkey 1: Read state
│       Per-channel last-read Lamport ts. Reconciles "unread" badges
│       across devices.
│
├── Subkey 2: Preferences
│       UI prefs that should follow the user (theme, notification levels,
│       quiet hours, default audio devices).
│
└── Subkey 3: Device list
        Cross-device authoritative list of paired devices with names,
        platforms, last-seen timestamps, push-relay routes.
```

Each subkey is **encrypted at rest** with a key derived from the master
secret (per-subkey HKDF info string). The DFLT schema means only the
master-secret holder can write — devices share the master secret via
the pairing flow below.

```rust
DHTSchema::DFLT { o_cnt: 4 }
```

The schema constant lives at
`rekindle_records::schema::personal_sync_dflt_schema`.

## Device pairing

Pairing is a short-lived `app_call` handshake gated by a one-time code.
The existing device displays the code (plus a QR encoding the full
triple); the new device transcribes or scans it and dials back.

```
        Existing device                         New device
        ─────────────────                       ──────────
                                                
1.  User: "Pair another device"
    ▶ generate_pairing_session()
        ├── code         ← random
        ├── salt         ← random
        └── insert into pending_pairings (TTL 5 min)
    ▶ Display code + QR(code‖salt‖record_key)
                                                 
2.                              ◀───── User scans QR
                                ◀───── (or types code + salt)
                                                 
3.                                       ▶ generate fresh device id
                                         ▶ derive PairingKey from
                                             HKDF(salt, code)
                                         ▶ build PairingPayload:
                                             { device_id, device_name,
                                               platform, ephemeral_pk }
                                         ▶ wrap with PairingKey
                                         ▶ app_call → existing device
                                                 
4.  ◀── handle_pairing_app_call(payload)
       ▶ verify code in pending_pairings (consume on use)
       ▶ derive same PairingKey
       ▶ unwrap payload
       ▶ wrap (master_secret, personal_record_key,
                owner_keypair_hex, device_list_subkey_index)
                with PairingKey
       ▶ reply with PairingAccept
                                                 
5.                                ─── PairingAccept ──▶
                                         ▶ unwrap → master secret +
                                             record key + owner keypair
                                         ▶ persist to local Stronghold
                                         ▶ open personal sync record
                                         ▶ append self to device list
                                                                 
6.  ▶ ValueChange on subkey 3 (device list)
    ▶ UI shows the new device
```

The pairing code is single-use and TTL-limited (5 min by default,
stored in the local `pending_pairings` table). The wrapped master
secret never appears on the wire in plaintext — `PairingKey` is derived
via HKDF from the code + salt, and an active eavesdropper would need
both the code and the salt and would still face an X25519 ECDH between
the devices' ephemeral keys.

QR codes encode the full `code ‖ salt ‖ personal_record_key` triple so
the user can pair without typing. Manual code entry is the fallback
when QR scanning is unavailable (e.g., across two desktops).

## Subkey I/O

Reads and writes go through `services/cross_device_sync/subkey_io.rs`
which handles the encrypt-then-write / read-then-decrypt:

```rust
write_sync_manifest(handle, plaintext)   // subkey 0
read_sync_manifest(handle)
write_read_state(handle, plaintext)      // subkey 1
read_read_state(handle)
write_preferences(handle, plaintext)     // subkey 2
read_preferences(handle)
write_device_list(handle, plaintext)     // subkey 3
read_device_list(handle)
```

Plaintext is Cap'n Proto-encoded. Each subkey has its own HKDF info
string so cross-subkey ciphertext-confusion is impossible. Veilid's
sequence-number write semantics handle conflict detection — a write
that arrives with a stale `seq` is rejected by the DHT, and the caller
re-reads, re-merges, and writes again.

## Watch-and-merge loop

```rust
start_personal_sync_watch(state, handle)
    └── watch_dht_values(record_key, full_subkey_range)
        ▶ on ValueChange:
            decrypt → CRDT-merge → apply to local state
            emit("sync-event") to frontend
        ▶ on watch lapse: re-arm; in the meantime inspect_dht_record
                          every 60 s as fallback
```

Merge rules per subkey:

| Subkey | Strategy | Notes |
|--------|----------|-------|
| 0 — manifest | LWW per logical entity (community, friend, DM thread) | Adds and removes are merged; ties broken by `(updated_at, device_id)`. |
| 1 — read state | Max-Register per channel | Highest known Lamport wins per channel. Matches the user's intuition: marking-read on any device "wins". |
| 2 — preferences | LWW per setting key | Same tiebreak as manifest. |
| 3 — device list | Grow-only set with explicit-removal tombstone | A device unpairs by writing a tombstone with its own `device_id`. |

The merge engine lives at `services/cross_device_sync/merge.rs` and is
exercised by the property tests in
`services/cross_device_sync/tests.rs`.

## Gap detection and history catch-up

Cross-device sync uses the same primitives as community catch-up:

- **`rekindle_sync::gap::GapDetector::detect(local, network)`** — given
  per-subkey local sequence numbers and the freshly inspected network
  values, return the list of subkeys where `network > local`.
- **`rekindle_sync::history::HistoryAd`** — a lightweight advert of
  which Lamport ranges a peer holds. Pickup logic in
  `select_best_peer` finds the peer with the widest range that covers a
  needed Lamport.
- **`FetchQueue`** — bounded retry queue with attempt counters; tasks
  drain into actual `get_dht_value` calls in the cross-cutting service
  layer.

These primitives are crate-private until the service layer plumbs them
in; the service layer composes them with Veilid I/O and SQLite.

## Record warming

Idle devices cycle through the personal record every 5 minutes
performing a `get_value` on subkey 0. This refreshes the DHT TTL and
keeps the record warm even when no actual updates are happening — see
[`communities.md` §8](communities.md#8-strand-relay--mutual-aid-patterns)
for the general pattern. Without warming, a quiet record can be
garbage-collected by the DHT after ~1 hour.

## Identity table integration

Every paired device persists the personal record key, the owner keypair
hex, and its own `device_id` in the local `identity` table:

```sql
identity:
  ...
  personal_sync_record_key   TEXT
  personal_sync_owner_keypair TEXT  -- hex; encrypted at rest by Stronghold
  device_id                  TEXT
  ...
```

This makes `ensure_personal_sync_record` idempotent — on subsequent
launches, the existing handle is reopened without going through the
pairing flow again.

## Where to look

| Concern | File |
|---------|------|
| Crate-level sync primitives | `crates/rekindle-sync/src/{fetch,gap,history,inspect,verify,warming,watch}.rs` |
| Personal record lifecycle | `src-tauri/src/services/cross_device_sync/record.rs` |
| Pairing handshake (both directions) | `src-tauri/src/services/cross_device_sync/pairing.rs` |
| Subkey encrypt/decrypt + read/write | `src-tauri/src/services/cross_device_sync/subkey_io.rs` |
| CRDT merge per subkey | `src-tauri/src/services/cross_device_sync/merge.rs` |
| Watch + inspect-poll loop | `src-tauri/src/services/cross_device_sync/watch.rs` |
| Subkey schema constants | `crates/rekindle-types/src/cross_device_sync.rs` |
| IPC commands | `src-tauri/src/commands/sync.rs` |

## Open work

- **Mobile platforms** — pairing flow exists but mobile-specific UX
  (deep-link handling, native QR scanner) is pending the mobile target.
- **Selective sync** — manifest currently includes every community and
  thread; future versions could let the user choose which threads
  follow them across devices vs which are device-local.
- **Device revocation** — a paired-device tombstone removes the device
  from the list, but does not yet re-key the record. A compromised
  device that already holds the master secret still has access to
  history. Master-secret rotation is a deferred design problem (it
  invalidates pseudonyms across communities).

Tracked in [`../roadmap.md`](../roadmap.md).

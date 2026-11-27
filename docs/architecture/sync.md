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

```text
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

```text
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
                                         ▶ persist to local vault
                                             (rekindle-vault)
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
            event_dispatch::emit_journaled("sync-event", …)
        ▶ on watch lapse: re-arm; in the meantime inspect_dht_record
                          every 60 s as fallback
```

The `sync-event` emit flows through the standard
[event dispatch router](event-dispatch.md), so a reload during a sync
update is recovered by the same `event_resume` cursor-tick mechanism
that recovers chat or presence events. Cross-device reconciliation
(the per-subkey CRDT merge) is a separate concern that runs on every
device's `ValueChange` independently — the journal does not
participate in the multi-device CRDT.

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

## A new friend's DM chain start (decision 67 item 2)

Each direction of a 1:1 conversation is its sender's chain on a dm participant record
(`operations::dm::stream`). A new friendship names both chains in the handshake itself, so neither
side's first DM waits on a `StreamRecord` `app_call` (finding F3, the Linux first-DM loss):

- **Request.** Once the requester's friend row exists, `friend::request::named_chain` makes its
  participant record locally (like the reply inbox) and puts its key in `FriendRequest.chainRecordKey`. Each attempt of the request's row,
  by `app_call` or into an invite's inbox, first writes the record's empty `HEAD`
  (`dm::first_contact_durable::chain_published`); a failed write defers the attempt. The accepter
  stores the key with the pending request (`pending_friend_requests.chain_record_key`) and adopts it
  only when it accepts (`friend::answer::accept`, or the crossing auto-accept), before its accept can
  reach the requester: the requester's chain is never watched before consent.
  A request that ends unanswered (cancelled, expired, or a declined one, which expires silently)
  or a removed friend releases the record as a removed friend's records are (`unwatch_friend`): the
  requester's vault forgets its writer (netsim `request_chain`).
- **Accept.** `dm::send_friend_accept` makes the accepter's record and names it, with the accepter's
  presence key, in the `FriendAccept` sealed inside the PreKeyInit; its `HEAD` is written before each
  attempt the same way. The requester applies both when the accept lands, by call or from its reply
  inbox (RV2).
- Neither side sends a `StreamRecord` for these records: they are held from the handshake on, so the
  first append finds them. Whichever side writes first, the other reads it from the chain it already
  watches (netsim `new_friend_chain`).
- Only an authenticated answer settles a handshake row (F2); the `HEAD` write never does.

## Group first-contact rendezvous

A group member's first contact to a member who is not its friend (the
PreKeyInit whose first payload is its `GroupDmAccept`, plan decision 67
item 3) has a durable path beside its `app_call`. The creator mints one
**group rendezvous record** per group (`SMPL(o_cnt 1, 8 one-time writers,
m_cnt 8)`, 65 subkeys; `dht_layout::group_rendezvous`): subkey
`1 + 8i + j` holds member `i`'s first contact to member `j`, writable only
by row `i`'s writer (Veilid's SMPL member range). Each member stores its
slot and its row's writer in `group_dm_rendezvous` at create or accept.

- **Write:** the first-contact row (`friend_handshake:<peer>`, named
  `PreKeyInit:`; `Framed::GroupFirstContact`) is attempted as the call and
  a write of the same datagram to `subkey(own slot, their slot)` through
  the record pool's republishing set (`write_own_once`). A frame over
  `MAX_SUBKEY_VALUE_LEN` (16,131 B) is refused when it is framed. Every
  retry calls and writes again; a B6 resend is a new init written over the
  row (`transport/operations/dm/group_first_contact.rs`).
- **Settle:** only an authenticated answer settles the row: the call's
  receipt, or the member's first frame opened under the session
  (`handshake::settle_first_contact`). The write's own outcome never does.
- **Read:** each member watches exactly its own column from accept to
  leave, re-held at unlock, and reads the column once whenever it holds
  it (a watch reports only later changes), so an entry written while it
  was locked or still invited lands (`rendezvous_watch.rs`). An entry is
  opened through `direct::receive_from_member` with the admission and the
  roster gate unchanged, and only from the member the creator's signed
  slot plan gives its row. A copy heard by both paths is dropped as a
  duplicate by the replay store or the ratchet, and a member's key already
  held is a `KeyUpdate::Duplicate`.

## Gap detection and history catch-up

Cross-device sync uses the same primitives as community catch-up:

- **The D23 stream engine (`rekindle_sync::stream`)** — a per-writer
  cursor; an entry past `cursor + 1` is a gap held for the diff, and a
  diff that cannot reach the writer's named head stores a gap marker
  (plan E3.3; the per-subkey `GapDetector` is deleted).
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
  personal_sync_owner_keypair TEXT  -- hex; encrypted at rest by the vault
  device_id                  TEXT
  ...
```

This makes `ensure_personal_sync_record` idempotent — on subsequent
launches, the existing handle is reopened without going through the
pairing flow again.

## Where to look

| Concern | File |
|---------|------|
| Crate-level sync primitives | `crates/rekindle-sync/src/{fetch,history,inspect,stream,verify,warming,watch}.rs` |
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

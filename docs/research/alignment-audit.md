# Alignment Audit — Tauri Thinness, Veilid, Chiral Architecture

Full-project audit against three questions:

1. Is the Tauri frontend thin and CLI-driven, as designed?
2. Is the project aligned with Veilid's actual API contract?
3. Is the chiral architecture complete?

Every claim below was traced end to end. Where a measurement was
suggestive but unproven it is marked as such, and the two that started
as leads were followed until they were either confirmed or dropped.

---

## 1. Tauri thinness — **not met**

**258 Tauri commands vs 64 daemon IPC variants.**

The daemon's variants were checked for coarse granularity first (a few
variants carrying nested op enums, which would make the counts
incomparable). They are flat — one per operation — so the surfaces are
directly comparable.

Matching by token set rather than substring (`join_community` ↔
`CommunityJoin` is a match; a substring test misses it):
**47 of 258 desktop commands have a daemon counterpart, 211 do not.**

| area | Tauri commands | no daemon equivalent |
|---|---|---|
| community | 144 | 111 |
| friends | 21 | 17 |
| calls | 13 | 13 |
| sync (device pairing) | 12 | 12 |
| voice | 12 | 10 |
| auth | 12 | 10 |
| dm | 9 | 7 |
| window | 8 | 7 |
| chat | 5 | 5 |

Some gaps are legitimate: `window` is a GUI concept, and native camera
capture and avatar data-URL rendering have no CLI meaning. But threads,
polls, events, reactions, pins, automod, onboarding, categories,
analytics, the audit log, attachments, expressions, device pairing, and
the **entire call subsystem** are features the CLI cannot reach at all.

### What is *not* wrong

`src-tauri` is 60k lines against 137k in crates, and 41k of that is
`services/`. That looks fat, but the breakdown does not support the
reading:

- adapters (the sanctioned Schwarzschild boundary): 11.8k
- runtimes: 6.7k
- `services/community/*`: 7.6k — and every large file there
  (`join/flow`, `video_session`, `message_notifications`,
  `channel_messages`, `gossip`, `files`, `mek_rotation`) delegates into
  a Tier-5/6/7 crate.

`services-pattern.md` is explicit that the harvest is incomplete by
design and names what remains. The defect is not fat orchestration in
the host — it is that **the daemon never received the surface**. The
desktop is not a thin frontend over the daemon; it is a parallel
implementation with four times the capability.

---

## 2. Veilid alignment — three verified defects

### 2.1 Sync response cannot fit, and goes to the wrong peer

`services/veilid/control_sync.rs::handle_sync_request` selects up to
`LIMIT 500` messages and packs them into a `SyncResponse`.

`SyncedMessage.sender_key` is a hex-encoded community pseudonym — 64
characters. **500 × 64 = 32,000 bytes of sender keys alone**, before
bodies (MEK-encrypted, so larger than plaintext), timestamps,
`mek_generation`, `lamport_ts`, JSON field names, or envelope overhead.

veilid-core 0.5.7, `rpc_processor/coders/operations/operation_app_message.rs`:

```rust
const MAX_APP_MESSAGE_MESSAGE_LEN: usize = 32768;
...
if message.len() > MAX_APP_MESSAGE_MESSAGE_LEN {
    return Err(RPCError::internal("AppMessage message too long to set"));
}
```

It **errors**; it does not truncate. So history catch-up for any channel
with a real backlog fails outright.

Second half: the response is sent with `send_to_mesh` — the gossip
fan-out — not to the peer that asked. The dispatcher **has**
`sender_pseudonym` in scope (`control_moderation.rs:19`) and threads it
into neighbouring handlers (lines 33, 47), but not into
`handle_sync_request`, whose signature has no requester parameter. So a
point-to-point history reply is broadcast to every member, multiplying
the oversize payload by the fan-out degree. The result is discarded with
`let _ =`, so nothing surfaces the failure.

Confirmed clean by comparison: file chunks are 28 KiB
(`CHUNK_SIZE_BYTES`) and video fragments 4 KiB
(`FRAGMENT_PAYLOAD_LIMIT`) — both comfortably under the limit.

### 2.2 A removed or blocked friend is still watched

Friends' profile records are opened read-only to read subkey 6 (route
blob) in `chat_runtime.rs`, `message_service/transport.rs` (×2), and
`presence_adapter/friend_deps.rs`, and watched via
`watch_friend_subkeys`.

Those keys are **never** registered in either inventory — not
`CommunityRecords` (community records only) and not the global
`open_records` set that `cleanup.rs::close_tracked_records` drains on
logout and app exit. A search for a friend `profile_dht_key` reaching
`track_open_records`, `store_dht_record`, `close_dht_record` or
`untrack_records` returns nothing.

veilid-core 0.5.7, `routing_context.rs` on `close_dht_record`:

> The release half of the open/close pair; **cancels the record's watch**
> (in the background) and drops any associated transaction.

Closing is what cancels a watch — and `friend_runtime/remove.rs` and
`friend_runtime/block.rs` contain no `watch`, `close_dht`, or `cancel`
call of any kind.

Consequences: we keep receiving presence for a peer we removed or
blocked, for the life of the process; and we keep occupying a watch slot
on *their* record, which Veilid bounds at `public_watch_limit: 32` per
record.

### 2.3 Relay routes leak on revoke

`services/relay/offer.rs` allocates a private route per volunteered
friend and persists `(friend → route_id, blob)` in
`strand_relay_volunteered`.

`revoke_relay` deletes the row and sends `RelayWithdraw` — and never
calls `release_private_route`. The `route_id` is dropped with the row.
A volunteer → revoke → volunteer cycle for one friend leaks one route
per iteration.

The personal route *is* released correctly (`cleanup.rs:60,154` via
`RouteManager::release_private_route`); this is specific to relay
routes.

---

## 3. Chiral architecture — one structural gap

**`src-tauri` never references `SubscriptionEvent`.** The Tauri frontend
receives `channels::presence_channel::PresenceEvent`
(FriendOnline / FriendOffline / StatusChanged / GameChanged,
`Serialize`-only); the CLI receives
`rekindle_types::subscription_events::PresenceEvent`
(CommunityMemberChanged / FriendChanged, round-trippable). Same for
`VoiceEvent`, and `CommunityDetail` on the view side.

Two event vocabularies, one per frontend. This is §1 seen from the other
end, and it is the mechanism: the desktop cannot be a thin frontend over
the daemon while it speaks a different event language.

Holding well: `cargo xtask check-boundaries` passes, all 35 crates sit
on the tier model, and the `Deps`/adapter pattern is applied
consistently where the harvest is done.

---

## Fix order

1. **2.1 sync response** — smallest change, clearest failure. Bound the
   response by bytes rather than row count, and thread `sender_pseudonym`
   through so the reply is directed.
2. **2.3 relay route leak** — one `release_private_route` in the revoke
   path, guarded by reading the stored `route_id` before the delete.
3. **2.2 friend watch** — track friend profile records in the global
   inventory, and close them on remove/block. Touches the friend
   lifecycle, so it wants its own test.
4. **§1 / §3 together** — the daemon surface and the event vocabulary
   are one piece of work, not two. Routing src-tauri's emit sites
   through the subscription stream is what makes the frontends
   interchangeable, and closing the 211-command gap is what makes the
   daemon the substrate. This is a phase, not a patch.

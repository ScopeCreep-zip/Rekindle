# Communities — Channels, Voice, DMs

This document covers the channel-level surfaces of Communities:
MEK lifecycle (the per-channel and per-community Media Encryption
Keys), channel messaging via `rekindle-channel`, voice / video /
stage channels, and direct messages (1:1 and group DMs).

The other two community docs are
[`communities-overview.md`](communities-overview.md) (model,
three-path delivery, SMPL schema, permissions, design principles)
and [`communities-governance.md`](communities-governance.md)
(CRDT governance, self-sovereign join, Plate Gate scaling).

## 1. MEK Lifecycle (Peer-to-Peer, No Vault)

The Message Encryption Key (MEK) protects channel content with
AES-256-GCM. There is no `MEKVault` DHT record, no
coordinator-owned key store, and no single point of failure. MEK
distribution is entirely peer-to-peer, implemented in
`crates/rekindle-mek-rotation/`.

### Distribution paths

| Event | Mechanism |
|-------|-----------|
| Community creation | Creator generates MEK (32 B from OS CSPRNG), writes `MEKGenerationBump { generation: 1 }`. MEK is **never** written to DHT. |
| Member joins (fresh invite) | MEK is included in `InviteSecrets`. Joiner verifies generation against governance. |
| Member joins (stale invite) | Joiner broadcasts `RequestMEK` via gossip; the deterministic responder replies via `app_call` with the wrapped key. |
| Member leaves / is banned | Remaining members rotate the MEK via the deterministic rotator protocol below. |

### Deterministic rotator selection

When a member departs, every remaining member independently
computes:

```rust
fn select_rotator(
    departed_pseudonym: &[u8; 32],
    remaining_members: &[[u8; 32]],
) -> [u8; 32] {
    remaining_members
        .iter()
        .min_by_key(|m| blake3::hash(&[departed_pseudonym, m].concat()))
        .copied()
        .unwrap()
}
```

The member with the lowest `blake3(departed || own_pseudonym)`
hash is the rotator. Same inputs → same output → no election, no
consensus, no coordinator.

The rotator:

1. Generates a new 32-byte MEK.
2. Increments the generation counter.
3. For each remaining member: derives a shared secret via X25519
   ECDH between their pseudonym keys (Ed25519 → X25519 birational
   map), encrypts the new MEK with XChaCha20-Poly1305.
4. Delivers the wrapped MEK to each member via `app_call`
   (confirmed delivery).
5. Broadcasts `MEKRotated { channel_id, generation, rotator_pseudonym }`
   via gossip.
6. Writes `MEKGenerationBump { generation, trigger_departed, cascade_skipped }`
   to the governance record.

### Reader validates the rotator too

`MEKGenerationBump` entries carry cryptographic proof of rotator
authority. Readers reject bumps from anyone who is not the
deterministic rotator (or the cascade successor if the legitimate
rotator was offline within a 30 s window). There is no "accept any
bump" path — invalid bumps are silently excluded by all honest
readers.

### Cascading selection

If the deterministic rotator is offline (crashed, kicked, or *was*
the departed member), members wait 30 s for `MEKRotated`. If not
received:

1. Each member excludes the failed rotator and recomputes.
2. The next-in-line rotator takes over, listing the failed rotator
   in `cascade_skipped`.
3. Readers verify each skipped member was offline (no presence
   heartbeat within the 30 s window) before accepting the cascade.

If all members' routes have expired (the community is fully
dormant), no rotation occurs — forward secrecy protects against
*future* eavesdropping by *departed* members, and a dormant
community has no future traffic. When any member returns, they
become the sole candidate and rotate immediately.

### Per-channel MEK

Each text channel has its own MEK with its own generation counter,
providing channel-level isolation: compromising one channel's MEK
does not compromise others. Voice channels rotate aggressively —
on every join and every leave — giving strong forward and backward
secrecy for live conversations.

See `crates-tier-detail.md` for the `rekindle-mek-rotation` module
surface and `../security/overview.md` for the full encryption
layer stack and how MEK fits alongside Signal Protocol (1:1 DMs)
and Veilid transport encryption.

## 2. Channel Messaging — `rekindle-channel`

The Tier-7 `rekindle-channel` crate hosts the channel messaging
pipeline: send, receive, threads, reactions, expressions
(custom emoji / soundboard / stickers), mentions, notifications,
polls, automod, slowmode.

The crate is layered on top of:

- `rekindle-mek-rotation` (per-channel MEK lookup),
- `rekindle-gossip` (mesh broadcast primitives),
- `rekindle-protocol` (signed envelope encoding),
- `rekindle-governance` (reader-validates permission checks),
- `rekindle-records` (SMPL write retry).

It is **pure logic**: it does not import `veilid-core` or `tauri`.
All I/O is plumbed in via the `ChannelMessagingDeps` trait,
implemented for the desktop app by
`src-tauri/src/services/channel_adapter/`. The adapter follows the
runtime / adapter / pure-logic split documented in
[`services-pattern.md`](services-pattern.md):

| Adapter submodule | Role |
|---|---|
| `deps_impl.rs` | The single `impl ChannelMessagingDeps` block — every method delegates to one of the submodules below |
| `state_reads.rs` | Channel / thread / member lookups, permission computation |
| `state_mutations.rs` | AppState writes (acquire lock, mutate, drop guard before await) |
| `dht.rs` | DHT writes / reads of channel messages, forwards, reactions, lazy-thread record creation |
| `persist.rs` | SQLite persists for messages / threads / sequences / slowmode state / retry-queue enqueue |
| `events.rs` | `ChannelEvent` → tracing + `ChatEvent` / `CommunityEvent` local echo + delivery state emits |

Tauri-side, channel messaging is invoked through `commands/community/messaging.rs`
(`send_channel_message`, `edit_channel_message`, `delete_channel_message`,
`forward_channel_message`, `admin_delete_channel_message`,
`bulk_delete_channel_messages`, `send_voice_message`) and surfaced
to the frontend through the `community-event` channel.

### Local persistence

Per-channel state has three storage layers:

| Surface | Purpose |
|---|---|
| `channels` SQLite table | Channel definition: `topic`, `slowmode_seconds`, `nsfw`, per-channel `message_record_key`, `mek_generation`, `log_key`, `my_sequence` (gap-detection counter) |
| `messages` SQLite table | Message rows, indexed by `(owner_key, conversation_id, timestamp)` with the dedup constraint |
| `src-tauri/src/channel_repo.rs` + `channel_materialize.rs` | Read / write façades the runtime uses; materialize produces denormalised snapshots for the UI |

The detailed schema for these tables is in
[`data-layer.md`](data-layer.md).

## 3. Voice, Video, and Stage

Voice traffic uses Veilid's `app_message` over the same 3-hop
Tor-class `SafetySelection::Safe` route as every other path, so the
sender's node identity is never exposed — voice gets no weaker
anonymity than text. It selects `Stability::LowLatency` within that
floor (lowest-latency anonymous variant), and the mouth-to-ear budget
is re-baselined to the ITU-T G.114 interactive band. The full voice
pipeline lives in `crates/rekindle-voice/`;
the Tauri-side orchestration is in `src-tauri/src/services/voice/`
(send / receive / MCU loops, signaling, device monitor, session
state). See [`voice.md`](voice.md) for the detailed pipeline.

### Mutual-aid SFU

| Channel size | Topology |
|--------------|----------|
| ≤ 4 members | Full-mesh P2P — every speaker sends every listener. |
| > 4 members | Mutual-aid SFU — the lowest-XOR-hash online peer acts as the relay (Selective Forwarding Unit). Decoding / encoding stays at the speaker; the SFU just fans out frames. |

The SFU role rotates as members come and go: the deterministic
selector (`min_by_key(blake3)`) picks the new SFU at each
membership change. No elections, no coordination — same input,
same output, every peer agrees.

### Stage channels

Discord-style speaker / audience model:

- Speakers have `SPEAK`; audience members do not.
- Audience members publish `ChannelEntry::HandRaise { raised: bool }`.
- Moderators promote raised hands by writing `RoleAssignment` for
  a "Speaker" role (or revoke `SPEAK` via `Unassignment`).

### Video and screen share

`rekindle-video` (Tier 7) handles fragmentation and reassembly.
The binding transport constraint is not the 32 KiB `app_message`
cap but veilid-tools' per-hop UDP segmentation: every envelope is
split into 1,272-byte fire-and-forget datagrams with all-or-nothing
reassembly and no retransmit, so large fragments rarely survive a
multi-hop route. Frames are therefore chunked into ≤ 4 KiB pieces
with Reed-Solomon parity and per-stream bounded reassembly buffers
(rationale documented in `crates/rekindle-video/src/fragment.rs`).
The codec (VP9 today) plugs in via a `VideoCodec` trait — the crate
handles only on-the-wire framing.

**Channel-scoped delivery.** Video media and its per-stream control
traffic (fragments, parity, acks, keyframe requests, bandwidth
estimates, topology changes, capability advertisements) never ride
the community gossip mesh. Senders address exactly the voice-channel
roster — the peers whose `VoiceJoin`/`VoiceRoster` signaling bound
them to the same channel — via directed per-peer `app_message` with
`ttl = 0` (`rekindle_gossip::send_to_channel_peers`). Three receiver
gates back this up (reader-validates):

1. Video envelopes are excluded from gossip forwarding even if a
   non-compliant sender ships them with `ttl > 0`.
2. `rekindle_video::handle_video_payload` drops any payload addressed
   to a channel the local user is not actively joined to, before
   reassembly, MEK decrypt, or any frontend event.
3. The §20.2 per-sender gossip rate floor (10 msg/s) exempts only
   media for the channel we are actively in — frame-rate traffic for
   any other channel stays under the floor and is then dropped by
   gate 2 anyway.

`MediaCapabilities` exchange survives the move off the mesh via
directed re-advertise hooks: every present member re-advertises to
the roster when a `VoiceJoin` for their channel arrives, and a joiner
re-advertises once its `VoiceRoster` lands.

Quality target today: ~480p @ 15 fps at ~800 kbps. Higher quality
requires upstream `veilid-media` work.

### Active calls (Phase 14.q)

`AppState.active_calls: Arc<dyn rekindle_calls::signaling::CallRegistry>`
holds the live registry of in-progress 1:1, DM, and group calls.
The trait surface lets `rekindle-calls` own the lifecycle decisions
(ring timeout, accept, decline, end); the adapter at
`src-tauri/src/services/calls_adapter/` owns the AppState writes
and the `event_dispatch` emits. Commands: `start_dm_call`,
`accept_dm_call`, `decline_dm_call`, `end_dm_call`,
`send_call_media_state`, `send_call_reaction`, `mute_caller_temp`,
plus the group-call variants.

## 4. DMs and Group DMs

DMs reuse the same SMPL infrastructure as community channels,
optimised for small private conversations. See `crates/rekindle-dm/`
for the pure-logic implementation; `src-tauri/src/services/dm/`
wires it to Veilid and SQLite via `DmStore` (the
`SqliteDmStore` impl is the production adapter).

### Direct messages (2 members)

```
SMPL Record (DM)
├── o_cnt: 0
├── member_count: 2
├── Subkey 0: Alice's messages (Alice writes)
├── Subkey 1: Bob's messages (Bob writes)
└── MEK: derived deterministically — no key exchange round-trip
```

The DM MEK comes from X25519 ECDH between the two identity keys:

```
dm_mek = HKDF-SHA256(
    ikm:  X25519(alice_private, bob_public),
    salt: SHA256(sorted(alice_pubkey || bob_pubkey)),
    info: b"rekindle-dm-mek-v1",
)
```

Both parties derive the same MEK independently. Ratcheted every
100 messages or 24 hours:
`mek_n+1 = HKDF(mek_n, "rekindle-dm-ratchet-v1")`.

Pseudonyms are **per-community**, so DMs initiated from
Community X use Alice's Community-X pseudonym and are unlinkable
to a DM in Community Y unless Alice voluntarily reveals it.

### Group DMs (3–8 members)

Same SMPL structure but with a *generated* MEK (ECDH is pairwise,
so a random MEK wrapped per-recipient is used instead).

Constraints:

- Maximum 8 participants — beyond that, create a community.
- No roles, no permissions, no governance. All members equal.
- One conversation, no channels, no threads.
- Any member can add up to the 8-member cap.
- MEK rotates on every leave (forward secrecy).

## Where the code lives

| Concern | Location |
|---|---|
| Pure channel-messaging crate | `crates/rekindle-channel/` |
| Pure DM / group DM crate | `crates/rekindle-dm/` |
| Pure call signaling crate | `crates/rekindle-calls/` |
| Voice audio pipeline | `crates/rekindle-voice/` |
| Video / screen-share framing | `crates/rekindle-video/` |
| MEK cascade rotation | `crates/rekindle-mek-rotation/` |
| File chunked delivery | `crates/rekindle-files/` |
| Channel adapter (Tauri) | `src-tauri/src/services/channel_adapter/` |
| DM adapter (Tauri) | `src-tauri/src/services/dm_adapter.rs`, `services/dm/` |
| Calls adapter (Tauri) | `src-tauri/src/services/calls_adapter/` |
| Voice adapter (Tauri) | `src-tauri/src/services/voice/`, `voice_adapter/` |
| MEK adapter (Tauri) | `src-tauri/src/services/mek_adapter.rs` |
| Channel commands | `src-tauri/src/commands/community/{messaging,channels,channel_admin,reactions_pins,threads,polls,expressions,files,link_previews}.rs` |
| DM commands | `src-tauri/src/commands/dm.rs` |
| Call commands | `src-tauri/src/commands/calls.rs` |
| Voice commands | `src-tauri/src/commands/voice.rs` |

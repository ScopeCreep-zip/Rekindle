# Communities — Chiral Network Overview

Rekindle Communities deliver Discord-class group chat — text channels,
voice, roles, permissions, threads, forums, events, reactions,
moderation, file sharing, rich presence — on the Veilid peer-to-peer
network with **zero servers, zero coordinators, and zero privileged
nodes**.

Every participating node is structurally identical. Governance is
not delegated; it is computed.

This document is the public spec for community contributors. It is
the overview half of a three-part split:

- This file — model, three-path delivery, SMPL schema, mutual-aid
  patterns, permissions, design principles.
- [`communities-channels.md`](communities-channels.md) — MEK
  lifecycle, channel messaging via `rekindle-channel`, voice / video /
  stage, DMs and group DMs.
- [`communities-governance.md`](communities-governance.md) — CRDT
  governance, self-sovereign join, Plate Gate scaling, the
  `rekindle-governance` + `rekindle-governance-runtime` split.

## 1. The Chiral Network Idea

The architecture borrows its mental model from *Death Stranding*'s
Chiral Network: every node is a porter, every node is a waystation.
There are no servers to provision, no coordinators to elect, no
leaders whose departure freezes the community. **Mutual aid is the
only infrastructure.** Members relay for each other because the
network is stronger when they do.

In practical terms:

- All community state lives in Veilid DHT records.
- Every record uses the **SMPL schema with `o_cnt: 0`** — zero
  owner-reserved subkeys. The Ed25519 keypair that creates the
  record is **shared infrastructure** distributed in invites, not a
  privileged credential.
- Each member writes only to their own subkey.
- Every reader independently merges all subkeys with deterministic
  CRDT rules to produce a consistent governance view.
- Permission enforcement is **reader-side**: any member can write
  any entry they like; honest readers drop entries the writer was
  not authorised to make.

This is **flat governance**: the community's truth is the union of
what its members have written, filtered by independent permission
validation.

### The Schwarzschild Principle

When a community is created, the Ed25519 keypair used to write the
genesis state collapses behind a horizon: the public key remains as
the community's permanent address, but the private key carries no
special governance authority.

Veilid still requires the owner keypair to *open* a record for
writing (`open_dht_record(key, Some(owner_keypair))`), so the
keypair is shared with all members via the invite. This is
analogous to a Death Stranding terminal: the Q-pid activates the
terminal, but the activation key is shared, not privileged.
**The keypair is an address key, not an authority key.**

### Why v2.0 replaced the coordinator model

The v1.0 architecture used a "rotating coordinator" pattern: the
community creator owned a DFLT manifest record, and operations like
channel CRUD or ban management were routed through whichever member
currently held the coordinator role. An audit found 15 critical
bugs (split-brain election, unrecoverable state on coordinator
hand-off, missing fencing tokens, dead heartbeat code, race
conditions) that were *symptoms of structural problems*:

- DFLT records have a single owner keypair — new coordinators
  *fundamentally cannot* write to the manifest.
- The DHT is eventually consistent, so coordinator election is
  split-brain by construction.
- No production P2P chat system uses dynamic leader election for
  group state (Session, Quiet, Briar, Matrix P2P, GossipSub all use
  either static authority or no coordinator at all).

v2.0 takes the no-coordinator path. Every operation that v1.0
routed through a coordinator is now a peer mesh operation resolved
by CRDT merge.

## 2. Three-Path Delivery

Every operation in a community travels three independent, redundant
paths. Any single path succeeding is sufficient. Together they
provide durability, speed, and consistency without any coordinating
node.

| Path | Mechanism | Latency | Durability | Available When |
|------|-----------|---------|------------|----------------|
| 1. **SMPL Write** | `set_dht_value()` to own subkey | 200–500 ms | Permanent (DHT) | DHT reachable |
| 2. **Gossip** | `app_message()` epidemic fan-out | 50–150 ms | Ephemeral | Online peers reachable |
| 3. **Watch / Inspect** | `watch_dht_values()` + 60 s `inspect_dht_record` | 1–60 s | N/A — reads Path 1 | DHT reachable |

**Path 1 is the source of truth.** If gossip delivers a message but
the DHT write fails, the message is *not* considered persisted. If
the DHT write succeeds but gossip fails, the message is still
durable — other members will discover it via Path 3.

**Path 2 provides "instant messaging" feel.** Online peers see new
messages within ~100 ms via gossip even before Path 1 finishes
propagating.

**Path 3 is consistency insurance.** `inspect_dht_record` is a
metadata-only call — it returns sequence numbers per subkey
without transferring payload bytes. The 60-second poll is nearly
free for idle channels but guarantees no update is permanently
missed if a watch lapses.

### The chiral notification model

Gossip carries **notifications**, not cargo. The wire payload is
metadata:

```
MessageNotification {
    channel_id, subkey_index, message_id,
    lamport_ts, sequence, content_hash,
}
```

The MEK-encrypted ciphertext lives only in the SMPL DHT record.
Recipients get the notification via gossip (fast) and fetch the
ciphertext from DHT (authoritative). This keeps ciphertext on
Veilid's ~5 storage replicas instead of 50–100+ gossip relay nodes
— a critical privacy property for vulnerable users facing
harvest-now-decrypt-later threats.

### Why not `app_call` for everything

During community bootstrap a new member triggers 30+ operations in
under 5 seconds. Each `app_call` holds a pending connection slot on
both sides; Veilid's connection table saturates and `TryAgain`
cascades cause join to fail. The three-path model uses `app_call`
only for operations that genuinely need a response (MEK delivery,
file chunk transfer, bootstrap bundle, call signaling). Everything
else uses SMPL writes and gossip, which are non-blocking.

## 3. The Universal SMPL Schema (Q-pid Equation)

Every multi-writer record in a v2.0 community uses the same schema:

```rust
DHTSchema::SMPL {
    o_cnt: 0,                                         // No owner subkeys
    members: vec![SMPLMember { m_max: 1, m_cnt: 1 }; 255],
}
```

- **255 member slots per record** (Veilid's practical SMPL limit
  before performance degrades). Communities exceeding 255 members
  use Plate-Gate segmentation
  ([`communities-governance.md`](communities-governance.md)).
- **One subkey per member per record.** A member's subkey index is
  the same across all records in the community: governance subkey
  7 = registry subkey 7 = channel subkey 7. Simplifies identity
  resolution.
- **Member keypairs are per-community** — distinct from the member's
  global Ed25519 identity, providing pseudonymity across
  communities.

A community is **3 + N** DHT records, where N is the number of
channels: bootstrap pointer (DFLT, optional), governance (SMPL),
member registry (SMPL), and one SMPL record per channel.

### Subkey overflow

Each subkey holds up to ~32 KiB. Three strategies prevent overflow:

| Record type | Growth | Mitigation |
|---|---|---|
| Governance | LWW entries supersede earlier ones | Compaction before write — keep only the latest entry per `entity_id`. Active admins compact to <8 KiB. |
| Registry | Single struct overwritten per heartbeat | Never grows. |
| Channel | Append-only message log | **Continuation chain** — when a subkey approaches 28 KiB, the member writes a `continuation_record_key` pointer and starts a new SMPL channel record. Readers cache the chain locally and skip cold records. |

Continuation rotation is the *timefall* pattern — old records age
into cold storage, new records carry live traffic, with no admin
action required.

## 4. Strand Relay & Mutual-Aid Patterns

Five infrastructure patterns make the chiral network self-healing
without any dedicated relay servers — *roads*, *shared lockers*,
*ziplines*, *porters*, *shelters*.

### Strand Relay Network

Friends volunteer as relay nodes for each other (architecture
§13). When Alice cannot reach Bob directly (stale route), she
sends through Carol — a mutual friend. **Carol cannot read the
content** (encrypted to Bob's key). Carol creates a dedicated
relay route (separate from her personal route), delivers the route
blob to Bob via `app_call`, and Bob publishes it in his relay
record (DFLT, owned by Bob's friend-profile key, padded with
dummies for unlinkability).

Privacy properties:

- Alice cannot identify which friend is relaying — opaque blobs,
  padded with dummies.
- Carol does not know who Alice is — the message arrives via
  Alice's private route.
- Content is encrypted to Bob only.

### Mutual-Aid Infrastructure

| Pattern | Purpose | Mechanism |
|---------|---------|-----------|
| **Record warming** | Keep DHT records cached in network nodes | Idle clients cycle every 5 minutes, performing `get_value` on subkey 0 of all community records. Refreshes TTL without payload. |
| **History advertisements** | Newcomers find peers who hold the messages they need | Members advertise `history_ranges: Vec<(channel_id, from_lamport, to_lamport)>` in `MemberPresence`. |
| **Watch relay** | Extend Veilid's limited per-record watch slots | Members with watch slots relay `ValueChange` notifications via gossip to watchless peers. |
| **Bootstrap bundles** | Single round-trip onboarding | One `app_call` returns governance + members + channel keys + current MEK + recent messages. Replaces 30+ DHT reads. |
| **Gossip topology optimisation** | Reliable paths emerge organically | Per-peer delivery metrics weight fan-out targets. High-reliability "ziplines" emerge from usage patterns, not central planning. |
| **MEK relay via gossip** | Stale joiners recover keys without flooding | `RequestMEK` propagates through the mesh; only the deterministic responder (lowest XOR distance) replies. |

Fan-out targets are weighted by score; reliable, low-latency paths
emerge without explicit topology management — the way porters in
Death Stranding wear paths through repeated delivery.

## 5. Permissions

Permissions use a Discord-compatible 64-bit bitmask (`u64`),
evaluated client-side from the merged CRDT state.

### Bitfield Layout

| Bits | Group | Examples |
|------|-------|----------|
| 0–15 | General | `VIEW_CHANNELS`, `MANAGE_CHANNELS`, `MANAGE_ROLES`, `MANAGE_COMMUNITY`, `CREATE_INVITES`, `KICK_MEMBERS`, `BAN_MEMBERS`, `TIMEOUT_MEMBERS` |
| 16–31 | Text | `SEND_MESSAGES`, `EMBED_LINKS`, `ATTACH_FILES`, `ADD_REACTIONS`, `MENTION_EVERYONE`, `MANAGE_MESSAGES`, `READ_HISTORY`, `PIN_MESSAGES` |
| 32–43 | Voice | `CONNECT`, `SPEAK`, `MUTE_MEMBERS`, `DEAFEN_MEMBERS`, `MOVE_MEMBERS`, `USE_VOICE_ACTIVITY`, `PRIORITY_SPEAKER`, `STREAM` |
| 44–47 | Threads | `MANAGE_THREADS`, `CREATE_PUBLIC_THREADS`, `CREATE_PRIVATE_THREADS`, `SEND_MESSAGES_IN_THREADS` |
| 48–49 | Events | `MANAGE_EVENTS`, `CREATE_EVENTS` |
| 50 | `ADMINISTRATOR` | Bypasses all permission checks. |

### Resolution Algorithm

For a `(member, channel)` pair, the effective permission set is:

1. **Community creator** → `ALL`.
2. **Start with `@everyone` role**.
3. **OR all member's role permissions**.
4. **If `ADMINISTRATOR`** → return `ALL`.
5. **`@everyone` channel overwrites** — `(perms & !deny) | allow`.
6. **Role channel overwrites** — union allows, then apply denies.
7. **Member-specific channel overwrites** — highest priority.
8. **Timeouts** — clamp to `VIEW_CHANNELS | READ_HISTORY` if timed out.
9. **Implicit dependencies** — no `SEND_MESSAGES` → drop dependents;
   no `VIEW_CHANNELS` → drop everything.

Every peer receiving a `CommunityEnvelope` runs the same algorithm
against the same merged CRDT state. Invalid messages are silently
dropped. A misbehaving client that ignores the rules only corrupts
its *own* view — every other peer independently validates.

## 6. Honest Tradeoffs vs. Discord

Rekindle is not a Discord drop-in. The architecture is fundamentally
different — peer-to-peer, end-to-end encrypted, no central server —
and that imposes real constraints.

| Feature | Rekindle | Discord | Tradeoff |
|---------|----------|---------|----------|
| Delivery latency | 50–350 ms | 20–50 ms | Comparable for chat; voice uses `Unsafe` for sub-50 ms |
| Search | Local FTS5 from join date | Server-side full history | Privacy: cannot search what you cannot decrypt |
| File availability | Peer-cached (≥1 online peer) | CDN | Dead community + no online peers = unreachable; local pinning mitigates |
| Push notifications | Opt-in relay (timing metadata leak) | Built-in | Three-tier escalation, each tier leaks more |
| Spam / AutoMod | Rate limit + governance ban + client filters | Server-side ML | Bursts before ban propagates (~2–5 s) |
| Max community size | 255 / segment, fractal scaling | 500 K+ | Plate-gate cap `MAX_SEGMENTS=8` ≈ 2 040 — deliberate boundary |
| Multi-device | Personal DHT sync record | Server-side session | 1–5 s sync latency |

### Where Rekindle wins

- **Privacy** — no server logs, no metadata collection, no IP
  correlation, unlinkable pseudonyms across communities.
- **Censorship resistance** — no central point to compel takedown,
  subpoena, or block.
- **Data ownership** — all data is local. No cloud breach surface,
  no platform ban.
- **Cost** — zero hosting. Communities are free forever.
- **Resilience** — community survives as long as members exist.
- **Surveillance resistance** — E2E + Veilid routing makes mass
  surveillance economically infeasible.

Rekindle suits communities that value privacy, censorship
resistance, and data ownership over convenience features.

## 7. Features Intentionally Omitted

Every omission below is a deliberate architectural decision:

- **Webhooks, server-hosted bots, Activities, Clyde AI** — no
  central infrastructure to host them.
- **OAuth2 / connected accounts** — linking a pseudonym to external
  services creates a correlation point that violates unlinkability.
- **Server Boost / premium tiers** — all communities are
  architecturally equal.
- **Server discovery** — a global directory would expose at-risk
  communities to enumeration. Discovery is opt-in only.
- **Phone / email verification** — identity is a keypair.
- **Coordinator / leader election** — DFLT records cannot rotate
  ownership; election is split-brain in eventually-consistent DHTs.
- **Vanity URLs, server-side audit log** — no DNS, no server.
  Governance entries in SMPL records are the distributed,
  tamper-evident audit trail (the [`audit chain`](audit-chain.md)
  is the local complement).

## 8. Design Principles

These principles govern every architectural decision, in order of
priority. When principles conflict, lower-numbered principles win.

1. **No node above another.** Every member is a full peer.
2. **One equation everywhere.** The universal SMPL schema is used
   for every multi-writer record.
3. **DHT is primary, gossip is secondary.** SMPL writes are the
   durable source of truth.
4. **Storage IS the vote.** DHT records expire if not refreshed;
   active use keeps them alive. No GC, no admin cleanup.
5. **Privacy is a stamina budget.** Voice uses `Unsafe`; text uses
   safety routes; governance uses 2–3 hops. A slider, not a switch.
6. **Assume everything degrades.** Three-path delivery exists
   because every individual path fails sometimes.
7. **Reader validates, not writer.** Any member can write any
   entry; readers independently check authority.
8. **Governance is replaceable; infrastructure is not.** SMPL,
   gossip, Veilid, encryption are load-bearing. Merge rules and
   policies can change without touching transport.
9. **Grow organically, not by plan.** Segments split fractal;
   ziplines emerge from usage; nothing centrally planned.
10. **All roads through Veilid.** No external transport.
11. **Honest about tradeoffs.** Every limitation is documented.
12. **Mutual aid is the incentive.** No tokens, no payment rails.
    Cooperative behaviours emerge from self-interest.

## Where this is implemented

| Concern | Location |
|---|---|
| Pure CRDT merge engine | `crates/rekindle-governance/` — Tier 6, no I/O, no async |
| Async governance lifecycle | `crates/rekindle-governance-runtime/` — Tier 6 |
| SMPL record lifecycle, retry queue | `crates/rekindle-records/` — Tier 3 |
| Gossip primitives (D-fanout, dedup, Lamport) | `crates/rekindle-gossip/` — Tier 5 |
| Private route lifecycle, peer cache | `crates/rekindle-route/` — Tier 4 |
| Signed envelope build / verify, dedup | `crates/rekindle-codec/` — Tier 3 |
| Cross-device sync, gap detection | `crates/rekindle-sync/` |
| Veilid integration (desktop) | `crates/rekindle-protocol/` |
| Unified Veilid boundary (daemon track) | `crates/rekindle-transport/` |
| Channel messaging, threads, reactions | `crates/rekindle-channel/` |
| Tauri shell, services, state, IPC | `src-tauri/` |
| Community services + adapters | `src-tauri/src/services/` — see [`services-pattern.md`](services-pattern.md) |
| Community IPC commands (~127, 32 modules) | `src-tauri/src/commands/community/` |
| Cap'n Proto schemas | `schemas/community.capnp` and siblings |

For deeper reference:

- [`overview.md`](overview.md) — full system stack, data-flow diagrams.
- [`crates.md`](crates.md) and [`crates-tier-detail.md`](crates-tier-detail.md) — every crate, its tier, its module layout.
- [`../protocol/overview.md`](../protocol/overview.md) — wire formats, Veilid primitives, DHT record layouts.
- [`../security/overview.md`](../security/overview.md) — encryption stack, identity, threat model.
- [`data-layer.md`](data-layer.md) — SQLite schema, vault layout, DHT record layout in storage terms.

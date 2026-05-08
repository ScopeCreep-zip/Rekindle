# Communities — Chiral Network Architecture (v2.0)

Rekindle Communities deliver Discord-class group chat — text channels, voice,
roles, permissions, threads, forums, events, reactions, moderation, file
sharing, rich presence — on the Veilid peer-to-peer network with **zero
servers, zero coordinators, and zero privileged nodes**.

Every participating node is structurally identical. Governance is not delegated;
it is computed.

This document is the public spec for community contributors. It distills the
internal design memo (`.claude/docs/rekindle-communities-architecture.md`) into
a reading order optimized for learning. For specific subsystems, cross-references
point to the dedicated docs in this directory.

---

## 1. The Chiral Network Idea

The architecture borrows its mental model from *Death Stranding*'s Chiral
Network: every node is a porter, every node is a waystation. There are no
servers to provision, no coordinators to elect, no leaders whose departure
freezes the community. **Mutual aid is the only infrastructure.** Members
relay for each other because the network is stronger when they do.

In practical terms:

- All community state lives in Veilid DHT records.
- Every record uses the **SMPL schema with `o_cnt: 0`** — zero owner-reserved
  subkeys. The Ed25519 keypair that creates the record is **shared
  infrastructure** distributed in invites, not a privileged credential.
- Each member writes only to their own subkey.
- Every reader independently merges all subkeys with deterministic CRDT rules
  to produce a consistent governance view.
- Permission enforcement is **reader-side**: any member can write any entry
  they like; honest readers drop entries the writer was not authorized to make.

This is **flat governance**: the community's truth is the union of what its
members have written, filtered by independent permission validation.

### The Schwarzschild Principle

When a community is created, the Ed25519 keypair used to write the genesis
state collapses behind a horizon: the public key remains as the community's
permanent address, but the private key carries no special governance authority.

Veilid still requires the owner keypair to *open* a record for writing
(`open_dht_record(key, Some(owner_keypair))`), so the keypair is shared with
all members via the invite. This is analogous to a Death Stranding terminal:
the Q-pid activates the terminal, but the activation key is shared, not
privileged. **The keypair is an address key, not an authority key.**

### Why v2.0 Replaced the Coordinator Model

The v1.0 architecture used a "rotating coordinator" pattern: the community
creator owned a DFLT manifest record, and operations like channel CRUD or
ban management were routed through whichever member currently held the
coordinator role. An audit found 15 critical bugs (split-brain election,
unrecoverable state on coordinator hand-off, missing fencing tokens, dead
heartbeat code, race conditions) that were *symptoms of structural problems*,
not bugs to fix:

- DFLT records have a single owner keypair — new coordinators *fundamentally
  cannot* write to the manifest.
- The DHT is eventually consistent, so coordinator election is split-brain
  by construction.
- No production P2P chat system uses dynamic leader election for group state
  (Session, Quiet, Briar, Matrix P2P, GossipSub all use either static authority
  or no coordinator at all).

v2.0 takes the no-coordinator path. Every operation that v1.0 routed through
a coordinator (channel CRUD, role management, bans, MEK rotation, join
approval, voice relay) is now a peer mesh operation resolved by CRDT merge.

---

## 2. Three-Path Delivery

Every operation in a community travels three independent, redundant paths.
Any single path succeeding is sufficient. Together they provide durability,
speed, and consistency without any coordinating node.

```
┌─────────────┐                              ┌─────────────┐
│   Sender    │                              │  Receiver   │
│             │──── PATH 1: SMPL Write ─────▶│ (DHT read)  │
│             │      set_dht_value()          │             │
│             │                              │             │
│             │──── PATH 2: Gossip ─────────▶│ (app_msg)   │
│             │      app_message() fan-out    │             │
│             │                              │             │
│             │                              │ PATH 3:     │
│             │        ┌──── DHT ────┐       │ watch fires │
│             │        │  subkey Δ   │──────▶│ or inspect  │
│             │        └─────────────┘       │ polls (60s) │
└─────────────┘                              └─────────────┘
```

| Path | Mechanism | Latency | Durability | Available When |
|------|-----------|---------|------------|----------------|
| 1. **SMPL Write** | `set_dht_value()` to own subkey | 200–500 ms | Permanent (DHT) | DHT reachable |
| 2. **Gossip** | `app_message()` epidemic fan-out | 50–150 ms | Ephemeral | Online peers reachable |
| 3. **Watch / Inspect** | `watch_dht_values()` + 60 s `inspect_dht_record` | 1–60 s | N/A — reads Path 1 | DHT reachable |

**Path 1 is the source of truth.** If gossip delivers a message but the DHT
write fails, the message is *not* considered persisted. If the DHT write
succeeds but gossip fails, the message is still durable — other members will
discover it via Path 3.

**Path 2 provides "instant messaging" feel.** Online peers see new messages
within ~100 ms via gossip even before Path 1 finishes propagating.

**Path 3 is consistency insurance.** `inspect_dht_record` is a metadata-only
call — it returns sequence numbers per subkey without transferring payload
bytes. The 60-second poll is nearly free for idle channels but guarantees no
update is permanently missed if a watch lapses.

### The Chiral Notification Model

Gossip carries **notifications**, not cargo. The wire payload is metadata:

```
MessageNotification {
    channel_id, subkey_index, message_id,
    lamport_ts, sequence, content_hash,
}
```

The MEK-encrypted ciphertext lives only in the SMPL DHT record. Recipients
get the notification via gossip (fast) and fetch the ciphertext from DHT
(authoritative). This keeps ciphertext on Veilid's ~5 storage replicas
instead of 50–100+ gossip relay nodes — a critical privacy property for
vulnerable users facing harvest-now-decrypt-later threats.

### Why Not `app_call` for Everything

Early v1.0 development reserved `app_call` (request-response) for Tier 2
operations, but during community bootstrap a new member triggers 30+
operations in under 5 seconds (governance fetch, registry read, channel list,
MEK request, presence announcement, history sync). Each `app_call` holds a
pending connection slot on both sides; Veilid's connection table saturates
and `TryAgain` cascades cause join to fail. The three-path model uses
`app_call` only for operations that genuinely need a response:

| Operation | Why `app_call` |
|-----------|----------------|
| MEK delivery | Sender must confirm recipient received the key |
| File chunk transfer | Per-chunk acknowledgment for flow control |
| Bootstrap bundle | Joiner needs a confirmed-complete snapshot |
| Call signaling | WebRTC-style offer/answer is request-response |

Everything else — chat, governance, presence, reactions, typing — uses SMPL
writes and gossip, which are non-blocking and do not consume connection slots.

---

## 3. The Universal SMPL Schema (Q-pid Equation)

Every multi-writer record in a v2.0 community uses the same schema:

```rust
DHTSchema::SMPL {
    o_cnt: 0,                                         // No owner subkeys
    members: vec![SMPLMember { m_max: 1, m_cnt: 1 }; 255],  // 255 slots, 1 subkey each
}
```

```
Subkey layout (universal for all records):
┌────────┬────────┬────────┬─────┬──────────┐
│ Sub 0  │ Sub 1  │ Sub 2  │ ... │ Sub 254  │
│ Mem 0  │ Mem 1  │ Mem 2  │     │ Mem 254  │
└────────┴────────┴────────┴─────┴──────────┘
  o_cnt=0: no owner subkeys, all belong to members
```

Properties:

- **255 member slots per record** (Veilid's practical SMPL limit before
  performance degrades). Communities exceeding 255 members use Plate-Gate
  segmentation (§7).
- **One subkey per member per record.** A member's subkey index is the same
  across all records in the community: governance subkey 7 = registry subkey
  7 = channel subkey 7. This simplifies identity resolution.
- **Member keypairs are per-community** — distinct from the member's global
  Ed25519 identity, providing pseudonymity across communities (§6).

### Record Layout

A community is **3 + N** DHT records, where N is the number of channels:

```
Community "Rekindle Dev"
├── Record 0: Bootstrap Pointer   (DFLT, optional, immutable)
├── Record 1: Governance          (SMPL o_cnt:0, 255 subkeys)
├── Record 2: Member Registry     (SMPL o_cnt:0, 255 subkeys)
├── Record 3: #general            (SMPL o_cnt:0, 255 subkeys)
├── Record 4: #development        (SMPL o_cnt:0, 255 subkeys)
└── ...
```

| Record | Schema | Purpose |
|--------|--------|---------|
| 0 — Bootstrap Pointer | DFLT, `o_cnt: 1`, immutable | Optional. Discovery only — points to records 1 and 2. Never mutated post-genesis. |
| 1 — Governance | SMPL, `o_cnt: 0` | Each member writes `GovernanceEntry` variants to their subkey. CRDT merged client-side. |
| 2 — Member Registry | SMPL, `o_cnt: 0` | Each member writes a single `MemberPresence` struct (overwritten on heartbeat). |
| 3+ — Channel Messages | SMPL, `o_cnt: 0` | Each member writes `ChannelEntry` variants (Message/Reaction/Edit/Delete/…) to their subkey. |

### Subkey Overflow

Each subkey holds up to ~32 KiB. Three strategies prevent overflow:

| Record Type | Growth | Mitigation |
|-------------|--------|------------|
| Governance | LWW entries supersede earlier ones | Compaction before write — keep only the latest entry per `entity_id`. Active admins compact to <8 KiB. |
| Registry | Single struct overwritten per heartbeat | Never grows. |
| Channel | Append-only message log | **Continuation chain** — when a subkey approaches 28 KiB, the member writes a `continuation_record_key` pointer and starts a new SMPL channel record. Readers cache the chain locally and skip cold records. |

Continuation rotation is the *timefall* pattern — old records age into cold
storage, new records carry live traffic, with no admin action required.

---

## 4. CRDT Governance

The merge engine lives in `rekindle-governance` (Tier 6). It is **pure
logic** — no I/O, no async, no side effects — so it is deterministically
testable and the same merge function runs identically on every peer.

### Merge Rules by Entry Type

Every governance entry includes a `lamport: u64` clock. The merge sorts all
entries from all subkeys by `(lamport, author_pseudonym)` for deterministic
total order, then applies type-specific rules:

| Entry Type | Strategy | Notes |
|------------|----------|-------|
| `ChannelCreated` / `ChannelArchived` | OR-Set | Active = created MINUS archived (matched by `channel_id`). |
| `RoleDefinition` | LWW-Register | Highest `lamport` wins per `role_id`. Ties: lexicographic pseudonym. |
| `RoleAssignment` / `RoleUnassignment` | LWW-Flag | Per `(target_pseudonym, role_id)`. |
| `BanEntry` / `UnbanEntry` | LWW-Flag | Per `target_pseudonym`. Unban requires higher `lamport` than the ban. |
| `MEKGenerationBump` | Max-Register | Highest `generation` is current MEK gen. Reader-validates rotator authority (§5). |
| `CommunityMeta` | LWW-Register | Single logical object — latest write replaces all fields. |
| `ThreadCreated` / `ThreadArchived` | OR-Set | `ThreadArchived` requires `MANAGE_THREADS`. |
| `EventCreated` / `EventArchived` | OR-Set | LWW within active set for metadata. |
| `ExpressionAdded` | OR-Set + tombstone | Removal via `AdminDelete` targeting the entry. |
| `CategoryCreated` / `CategoryArchived` | OR-Set | Same as channels. |
| `PermissionOverwrite` | LWW-Register | Per `(channel_id, target_id)`. |
| `AutoModRule` | LWW-Register | Per `rule_id`. |
| `SegmentAdded` | Grow-Only Set | Plate-Gate segments accumulate (§7). |

For channel records:

| Entry | Strategy | Notes |
|-------|----------|-------|
| `Message` | Grow-Only Set | All messages from all subkeys included; ordered by `(lamport, pseudonym)`. |
| `Edit` | Author-LWW | Edits applied only if the edit's author matches the original message author. |
| `Delete` | Tombstone | Permanent. Removes message from materialized view. |
| `Reaction` | PN-Counter | Distinct pseudonyms with `added: true` at their latest `lamport`. |
| `PollVote` | LWW-Flag | Per `(poll_id, pseudonym)`. |

### The Reader-Validates Principle

```
Writer writes GovernanceEntry::ChannelCreated to their subkey
  ↓
Reader reads all subkeys
  ↓
Reader merges CRDT state to determine current roles
  ↓
Reader checks: does writer's pseudonym hold a role with MANAGE_CHANNELS?
  ↓
  YES → entry included in materialized view
  NO  → entry silently ignored
```

Consequences:

- A member without `MANAGE_CHANNELS` *can* write `ChannelCreated`. Honest
  clients ignore it.
- A banned member can keep writing to their subkey forever. Honest clients
  drop everything they wrote after the ban entry's `lamport`.
- Security comes from *every reader independently validating*, not from
  preventing writes. The SMPL schema gives every member irrevocable write
  access to their own subkey by construction.

### Genesis Validation

Entries at Veilid sequence 1 (the first write to a subkey) are **always
accepted** regardless of permissions. This bootstraps the community: the
creator writes initial roles, channels, and metadata in their genesis entries
before any permission structure exists.

### Circular Dependency Resolution

Role assignments determine who can make governance changes, but role
assignments are themselves governance changes. Resolution is by Lamport
order: the merge processes entries in chronological order, and at each entry
the *currently accumulated* permission state determines validity. Genesis
entries bypass this check.

```rust
fn merge_governance(subkeys: &[SubkeyPayload]) -> GovernanceState {
    let mut state = GovernanceState::default();
    let mut all_entries: Vec<(PublicKey, &GovernanceEntry)> = Vec::new();

    for payload in subkeys {
        for entry in &payload.entries {
            all_entries.push((payload.pseudonym, entry));
        }
    }
    // Deterministic total order
    all_entries.sort_by_key(|(pk, e)| (e.lamport(), *pk));

    for (idx, (author, entry)) in all_entries.iter().enumerate() {
        let is_genesis = idx == 0;
        if is_genesis || state.has_permission(author, entry.required_permission()) {
            state.apply(author, entry);
        }
        // else: silently ignored
    }
    state
}
```

The merge function is property-tested in `rekindle-governance/proptest-regressions/`
for convergence (same entries in any order → same state), idempotence
(applying same entry twice → same state), and commutativity.

---

## 5. MEK Lifecycle (Peer-to-Peer, No Vault)

The Message Encryption Key (MEK) protects channel content with AES-256-GCM.
There is no `MEKVault` DHT record, no coordinator-owned key store, and no
single point of failure. MEK distribution is entirely peer-to-peer.

### Distribution Paths

| Event | Mechanism |
|-------|-----------|
| Community creation | Creator generates MEK (32 B from OS CSPRNG), writes `MEKGenerationBump { generation: 1 }`. MEK is **never** written to DHT. |
| Member joins (fresh invite) | MEK is included in `InviteSecrets`. Joiner verifies generation against governance. |
| Member joins (stale invite) | Joiner broadcasts `RequestMEK` via gossip; the deterministic responder replies via `app_call` with the wrapped key. |
| Member leaves / is banned | Remaining members rotate the MEK via the deterministic rotator protocol (below). |

### Deterministic Rotator Selection

When a member departs, every remaining member independently computes:

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

The member with the lowest `blake3(departed || own_pseudonym)` hash is the
rotator. Same inputs → same output → no election, no consensus, no
coordinator. The rotator:

1. Generates a new 32-byte MEK.
2. Increments the generation counter.
3. For each remaining member: derives a shared secret via X25519 ECDH between
   their pseudonym keys (Ed25519 → X25519 birational map), encrypts the new
   MEK with XChaCha20-Poly1305.
4. Delivers the wrapped MEK to each member via `app_call` (confirmed delivery).
5. Broadcasts `MEKRotated { channel_id, generation, rotator_pseudonym }` via
   gossip.
6. Writes `MEKGenerationBump { generation, trigger_departed, cascade_skipped }`
   to the governance record.

### Reader Validates the Rotator Too

`MEKGenerationBump` entries carry cryptographic proof of rotator authority.
Readers reject bumps from anyone who is not the deterministic rotator (or
the cascade successor if the legitimate rotator was offline within a 30 s
window). There is no "accept any bump" path — invalid bumps are silently
excluded by all honest readers, just like any other governance entry.

### Cascading Selection

If the deterministic rotator is offline (crashed, kicked, or *was* the
departed member), members wait 30 s for `MEKRotated`. If not received:

1. Each member excludes the failed rotator and recomputes.
2. The next-in-line rotator takes over, listing the failed rotator in
   `cascade_skipped`.
3. Readers verify each skipped member was offline (no presence heartbeat
   within the 30 s window) before accepting the cascade.

If all members' routes have expired (the community is fully dormant), no
rotation occurs — forward secrecy protects against *future* eavesdropping by
*departed* members, and a dormant community has no future traffic. When any
member returns, they become the sole candidate and rotate immediately.

### Per-Channel MEK

Each text channel has its own MEK with its own generation counter, providing
channel-level isolation: compromising one channel's MEK does not compromise
others. Voice channels rotate aggressively — on every join and every leave —
giving strong forward and backward secrecy for live conversations.

See `security.md` for the full encryption layer stack and how MEK fits
alongside Signal Protocol (1:1 DMs) and Veilid transport encryption.

---

## 6. Self-Sovereign Join

Joining requires no coordinator approval, no online ceremony beyond DHT reads
and a single slot claim. The joiner has all cryptographic material in the
invite and performs all operations independently.

### Invite Structure

An invite is an encrypted blob distributed out-of-band (deep link, QR code,
peer share). The decryption key is encoded in the deep link URL fragment
(never sent to any server):

```
rekindle://invite/{base64url(encrypted_blob)}#{base64url(decryption_key)}
```

The decrypted `InviteSecrets`:

```rust
struct InviteSecrets {
    governance_key: TypedKey,
    registry_key: TypedKey,
    slot_seed: [u8; 32],                         // For deriving slot keypairs
    channel_keys: Vec<ChannelKeyInfo>,
    current_mek: HashMap<TypedKey, MekInfo>,     // (channel, mek, generation)
    community_name: String,
    inviter_pseudonym: [u8; 32],
    inviter_route_blob: Vec<u8>,                 // For BootstrapBundle request
}
```

### Join Sequence

| Step | Action | Notes |
|------|--------|-------|
| 1 | Decrypt invite | XChaCha20-Poly1305 keyed by URL fragment. |
| 2 | Parse `InviteSecrets` | Cap'n Proto. |
| 3 | Request `BootstrapBundle` | `app_call` to inviter. **Convenience, not trust.** |
| 4 | Open governance record, build state | CRDT merge of all subkeys. Verifies the bundle. |
| 5 | Derive pseudonym | HKDF(`master_secret`, `community_id`) → Ed25519 keypair. Unlinkable across communities. |
| 6 | Check ban list | Banned pseudonyms abort the join client-side. |
| 7 | Scan registry for empty slot | `inspect_dht_record` returns subkey seqs; lowest-indexed seq=0 is empty. |
| 8 | Derive slot keypair, write `MemberPresence` | HKDF(`slot_seed`, `subkey_index`) → Veilid keypair. |
| 9 | Verify claim (compare-and-swap) | Re-read with `force_refresh=true`. On conflict, retry next slot (max 5). |
| 10 | Request current MEK if invite is stale | `RequestMEK` gossip; deterministic responder replies. |
| 11 | Open all channel records | `open_dht_record` + `watch_dht_values`. |
| 12 | Bootstrap gossip peers | Read every occupied registry slot; pick D online peers as initial neighbors. |
| 13 | Watch all records | Governance, registry, every channel. |
| 14 | Start presence heartbeat | 15 s interval, refreshes route blob. |
| 15 | SMPL catchup | Read all subkeys of all channel records, merge-sort, decrypt, store in SQLite. Background. |

### BootstrapBundle: Porter Delivery

The bundle is a single `app_call` to the inviter that returns:

- Pre-merged governance entries.
- Online member list with route blobs.
- Channel MEKs wrapped per-channel for the joiner via X25519 ECDH.
- Last 50 messages per channel (MEK-encrypted ciphertext).
- The owner keypair wrapped for the joiner.

Saves ~30 independent DHT reads, reducing join time from ~10–30 s to ~1–2 s.
The joiner verifies the bundle against DHT reads — if anything mismatches,
the DHT is authoritative. The bundle is cargo delivered by a porter; the
recipient verifies the seal.

If the inviter is offline, the joiner falls through to direct DHT reads. The
join is self-sovereign regardless of inviter availability.

### Leave and Rejoin

Leaving is unilateral: stop heartbeat, zero own registry slot, close records,
optionally delete local SQLite data. No coordinator approval. The slot
becomes available for reuse. A leaver who rejoins via a new invite derives
the *same* pseudonym (deterministic from `master_secret` + `community_id`)
but claims a new slot — their previous messages remain attributed to the
same identity.

---

## 7. Plate-Gate Scaling (Past 255 Members)

A single SMPL DHT record holds 255 member subkeys. Communities larger than
that are split into **fractal segments** — additional registry + governance
records announced via `GovernanceEntry::SegmentAdded`.

```rust
GovernanceEntry::SegmentAdded {
    segment_index: u16,
    governance_key: TypedKey,
    registry_key: TypedKey,
    slot_range: (u16, u16),  // member slot range this segment covers
    lamport: u64,
}
```

| Community Size | Registry Segments | Governance Segments | Channel Segments |
|----------------|-------------------|---------------------|------------------|
| ≤ 255 | 1 | 1 | 1 per channel |
| 256–510 | 2 | 2 | On demand (lazy, deferred to C1-2) |
| 511–765 | 3 | 3 | On demand |
| ~1000 | 4 | 4 | On demand |

### CRDT Mechanics

The CRDT model is an **ORMap-of-CRDTs** (Shapiro 2011, Almeida 2016): each
segment is its own join-semilattice; the community state is the product CRDT
under coordinate-wise join. **Cross-segment invariants are reader-validated,
never written into per-segment state.** Every peer fetches each segment's
author entries and runs the same `rekindle_governance::merge` over the union.

### What Ships Today (C1)

| Concern | Mechanism |
|---------|-----------|
| Membership discovery | `services/community/segments.rs::segment_descriptors` lists every active segment from merged governance state. Presence poll iterates all segments. |
| Admin expansion | `expand_community_segment` writes a `SegmentAdded` entry that creates the new SMPL records. |
| Slot claim | `services/community/join/flow.rs` walks segment descriptors in order, claims the first free slot in any segment. |
| Governance fetch | `commands/auth.rs::rebuild_governance_from_dht` does a two-pass merge: primary segment first, then every additional segment. CRDT idempotence makes the second pass safe. |
| Gossip | Crosses segment boundaries naturally — keyed by `(community_id, channel_id)`, not by segment. |
| Hard cap | `MAX_SEGMENTS = 8` (≈2,040 members) — soft cap; raising the constant lifts the limit at the cost of read amplification on presence poll. |

### Channel Records and C1-2

Channel records are themselves segmented at scale, but lazy — created only
when the first member of a new segment writes to that channel. Lazy
per-segment channel records ship in **C1-2** along with cross-segment MEK
distribution and the `ChannelSegmentLinked` governance entry.

Until C1-2 lands, communities that have expanded past one segment have two
behaviors:

1. **Online recipients:** Gossip carries every message regardless of segment,
   so live conversations work everywhere.
2. **Offline recipients in segments ≥ 1:** They will not catch up via Path 1
   until C1-2 introduces per-segment channel records.

The alternative — bridging via a designated segment-0 relay peer — would
reintroduce a single point of failure of exactly the kind v2.0 was built to
remove. Lazy per-segment channel records are the spec-mandated route, and
shipping a relay-bridge stopgap would make landing the real solution harder.

External references: Shapiro 2011 *Conflict-Free Replicated Data Types*,
Almeida 2016 *Delta State Replicated Data Types* (arXiv:1603.01529), Riak
DT, Matrix faster-joins, Discord guild sharding.

---

## 8. Strand Relay & Mutual-Aid Patterns

Five infrastructure patterns make the chiral network self-healing without
any dedicated relay servers. In Death Stranding terms: *roads*, *shared
lockers*, *ziplines*, *porters*, *shelters*.

### Strand Relay Network

Friends volunteer as relay nodes for each other (architecture spec §13).
When Alice cannot reach Bob directly (stale route), she sends through
Carol — a mutual friend. **Carol cannot read the content** (it is encrypted
to Bob's key).

```
Alice ──▶ Carol's relay route ──▶ Bob
       (RelayEnvelope, encrypted-to-Bob)
```

Setup: Carol creates a dedicated relay route (separate from her personal
route), delivers the route blob to Bob via `app_call`, and Bob publishes it
in his relay record (DFLT, owned by Bob's friend-profile key, padded with
dummies for unlinkability).

Privacy properties:

- Alice cannot identify which friend is relaying — opaque blobs, padded with
  dummies.
- Carol does not know who Alice is — the message arrives via Alice's private
  route.
- Content is encrypted to Bob only.

| Path | Latency |
|------|---------|
| Direct route alive | 50–150 ms |
| Stale route via relay | 60–100 ms |
| DHT fallback (no relay) | 200–500 ms |

### Mutual-Aid Infrastructure

| Pattern | Purpose | Mechanism |
|---------|---------|-----------|
| **Record warming** | Keep DHT records cached in network nodes | Idle clients cycle every 5 minutes, performing `get_value` on subkey 0 of all community records. No payload is fetched — just refreshes TTL. |
| **History advertisements** | Newcomers find peers who hold the messages they need | Members advertise `history_ranges: Vec<(channel_id, from_lamport, to_lamport)>` in `MemberPresence`. |
| **Watch relay** | Extend Veilid's limited per-record watch slots | Members with watch slots relay `ValueChange` notifications via gossip to watchless peers. |
| **Bootstrap bundles** | Single round-trip onboarding | One `app_call` returns governance + members + channel keys + current MEK + recent messages. Replaces 30+ DHT reads. |
| **Gossip topology optimization** | Reliable paths emerge organically | Per-peer delivery metrics weight fan-out targets. High-reliability "ziplines" emerge from usage patterns, not central planning. |
| **MEK relay via gossip** | Stale joiners recover keys without flooding the network | `RequestMEK` propagates through the mesh; only the deterministic responder (lowest XOR distance) replies. |

The peer-reliability metrics that drive zipline emergence:

```rust
struct PeerReliability {
    messages_forwarded: u64,
    messages_dropped: u64,    // inferred from gaps in Lamport sequences
    avg_latency_ms: f64,
    last_seen: Timestamp,
}
```

```
score = (messages_forwarded / (messages_forwarded + messages_dropped + 1))
        * (1.0 / (avg_latency_ms + 1.0))
```

Fan-out targets are weighted by score. Reliable, low-latency paths emerge
without explicit topology management — exactly the way porters in Death
Stranding wear paths through repeated delivery.

---

## 9. Permissions

Permissions use a Discord-compatible 64-bit bitmask (`u64`), evaluated
client-side from the merged CRDT state.

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

For a `(member, channel)` pair, the effective permission set is computed:

1. **Community creator** → `ALL`.
2. **Start with `@everyone` role**.
3. **OR all member's role permissions**.
4. **If `ADMINISTRATOR`** → return `ALL`.
5. **`@everyone` channel overwrites** — `(perms & !deny) | allow`.
6. **Role channel overwrites** — union allows, then apply denies.
7. **Member-specific channel overwrites** — highest priority.
8. **Timeouts** — clamp to `VIEW_CHANNELS | READ_HISTORY` if timed out.
9. **Implicit dependencies**:
   - No `SEND_MESSAGES` → drop `MENTION_EVERYONE`, `ATTACH_FILES`, `EMBED_LINKS`.
   - No `VIEW_CHANNELS` → drop everything (set to 0).
   - No `CONNECT` → drop `SPEAK`, `MUTE_MEMBERS`, `DEAFEN_MEMBERS`,
     `USE_VOICE_ACTIVITY`, `PRIORITY_SPEAKER`, `STREAM`.

### Reader Validates

Every peer receiving a `CommunityEnvelope` runs the same algorithm against
the same merged CRDT state. Invalid messages are silently dropped:

| Wire variant | Required permission |
|--------------|---------------------|
| `ChatMessage` | `SEND_MESSAGES` (or `SEND_MESSAGES_IN_THREADS`) |
| `Reaction` | `ADD_REACTIONS` |
| `PinMessage` | `PIN_MESSAGES` or `MANAGE_MESSAGES` |
| `DeleteMessage` (other's) | `MANAGE_MESSAGES` |
| `ChannelCreated` | `MANAGE_CHANNELS` |
| `RoleDefinition` | `MANAGE_ROLES` |
| `BanEntry` | `BAN_MEMBERS` |
| Etc. | Per-entry mapping in `rekindle-governance::permissions` |

A misbehaving client that ignores the rules only corrupts its *own* view —
every other peer independently validates.

---

## 10. Voice, Video & Stage

Voice traffic uses Veilid's `app_message` with `SafetySelection::Unsafe` for
sub-50 ms latency, accepting reduced sender anonymity (acceptable in voice
channels where participants are known).

### Mutual-Aid SFU

| Channel size | Topology |
|--------------|----------|
| ≤ 4 members | Full-mesh P2P — every speaker sends every listener. |
| > 4 members | Mutual-aid SFU — the lowest-XOR-hash online peer acts as the relay (Selective Forwarding Unit). Decoding/encoding stays at the speaker; the SFU just fans out frames. |

The SFU role rotates as members come and go: the deterministic selector
(`min_by_key(blake3)`) picks the new SFU at each membership change. No
elections, no coordination — same input, same output, every peer agrees.

### Stage Channels

Discord-style speaker/audience model:

- Speakers have `SPEAK`; audience members do not.
- Audience members publish `ChannelEntry::HandRaise { raised: bool }`.
- Moderators promote raised hands by writing `RoleAssignment` for a "Speaker"
  role (or revoke `SPEAK` via `Unassignment`).

### Video & Screen Share

`rekindle-video` (Tier 7) handles fragmentation and reassembly. Per the
Veilid `app_message` 32 KiB cap, frames are chunked into ≤ 28 KiB pieces
with FEC-friendly indexing and per-stream bounded reassembly buffers. The
codec (VP9 today) plugs in via a `VideoCodec` trait — the crate handles only
on-the-wire framing.

Quality target today: ~480p @ 15 fps at ~800 kbps. Higher quality requires
upstream `veilid-media` work (Phase 8+).

---

## 11. DMs and Group DMs

DMs reuse the same SMPL infrastructure as community channels, optimized for
small private conversations. See `rekindle-dm` for the implementation.

### Direct Messages (2 members)

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

Both parties derive the same MEK independently. Ratcheted every 100 messages
or 24 hours: `mek_n+1 = HKDF(mek_n, "rekindle-dm-ratchet-v1")`.

Pseudonyms are **per-community**, so DMs initiated from Community X use
Alice's Community-X pseudonym and are unlinkable to a DM in Community Y unless
Alice voluntarily reveals it.

### Group DMs (3–8 members)

Same SMPL structure but with a *generated* MEK (ECDH is pairwise, so a random
MEK wrapped per-recipient is used instead). Constraints:

- Maximum 8 participants — beyond that, create a community.
- No roles, no permissions, no governance. All members are equal.
- One conversation, no channels, no threads.
- Any member can add up to the 8-member cap.
- MEK rotates on every leave (forward secrecy).

---

## 12. Honest Tradeoffs vs. Discord

Rekindle is not a Discord drop-in. The architecture is fundamentally
different — peer-to-peer, end-to-end encrypted, no central server — and that
imposes real constraints.

| Feature | Rekindle | Discord | Tradeoff |
|---------|----------|---------|----------|
| Delivery latency | 50–350 ms (gossip + Veilid relays) | 20–50 ms (server push) | Comparable for chat; voice uses `Unsafe` for sub-50 ms. |
| Search | Local FTS5 from join date | Server-side full history | Privacy: you cannot search what you cannot decrypt. |
| File availability | Peer-cached (≥1 online peer with the file) | CDN | A file in a dead community with no online peers is unreachable. Local pinning mitigates. |
| Push notifications | Opt-in relay (timing metadata leak) | Built-in | Three-tier escalation (foreground / background fetch / opt-in relay), each tier leaks more. |
| Spam/AutoMod | Rate limit (~5 msg / 10 s) + governance ban + client filters | Server-side ML | Bursts of ~5 messages possible before ban propagates (~2–5 s). |
| Message deletion | Tombstone (request, not guarantee) | Server-permanent | Inherent to E2E P2P — old ciphertext may be cached. |
| Video quality | ~480p @ 15 fps interim | Up to 4K @ 60 fps | Veilid `app_message` is small-payload; higher quality needs upstream `veilid-media`. |
| Max community size | 255 / segment, fractal scaling | 500 K+ | Plate-gate cap (`MAX_SEGMENTS=8` ≈ 2,040). Discord-scale is architecturally incompatible with full-mesh gossip — deliberate boundary. |
| Moderation speed | 1–5 s gossip propagation | <100 ms server | Banned user can send 1–3 more messages before the ban catches up — readers retroactively filter. |
| Content filtering | Client-side advisory only | Server-side ML | Impossible with E2E by design. Communities self-govern. |
| Offline state changes | Queued in local SQLite, merged on reconnect | Server processes immediately | CRDT merge handles convergence on reconnection. |
| Message history | Local only, from join date | Full server-side, any device | `BootstrapBundle` ships ~500 messages/channel; older requires online peers who hold it. |
| Uptime | Requires ≥ 1 online member for real-time chat | 99.99 % SLA | DHT records persist ~1 hour without refresh; Strand Relay bridges gaps. |
| API / integrations | Headless member protocol only | REST + webhooks + bots | Bots join as headless members — more secure, less ecosystem. |
| Multi-device | Personal DHT sync record | Server-side session | 1–5 s sync latency over DHT. |

### Where Rekindle Wins

| Feature | Advantage |
|---------|-----------|
| Privacy | No server logs, no metadata collection, no IP correlation, unlinkable pseudonyms across communities. |
| Censorship resistance | No central point to compel takedown, subpoena, or block. |
| Data ownership | All data is local. No cloud breach surface. No platform ban. |
| Cost | Zero hosting. Communities are free forever. |
| Resilience | No single point of failure. Community survives as long as members exist. |
| Surveillance resistance | E2E + Veilid routing makes mass surveillance economically infeasible. |
| Platform lock-in | Open protocol, MIT license. Data is portable; clients are interchangeable. |

Rekindle suits communities that value privacy, censorship resistance, and
data ownership over convenience features. Gaming friend groups (5–50),
activist communities, journalist networks, privacy-conscious users will find
the tradeoffs acceptable. Communities that need 100 K+ members, server-side
bots, instant push, or 4K video should use Discord — that is a legitimate,
non-judgmental recommendation.

---

## 13. Features Intentionally Omitted

Every omission below is a deliberate architectural decision, not a missing
feature. The most load-bearing reasons:

| Feature | Reason |
|---------|--------|
| Webhooks | No HTTP endpoint exists — there is no server to receive payloads. |
| Server-hosted bots | No central hosting. Bots join as headless members via the same protocol. |
| OAuth2 / connected accounts | Linking a pseudonym to GitHub/Twitch/Steam creates a correlation point that violates unlinkability. |
| Server Boost / premium tiers | All communities are architecturally equal. No resources to "boost". |
| Server discovery (automatic) | A global directory would expose at-risk communities to enumeration. Discovery is opt-in only. |
| Phone / email verification | Identity is a keypair, not an account. Verification would require a central service. |
| Coordinator / leader election | DFLT records cannot rotate ownership; election is split-brain by construction in eventually-consistent DHTs. |
| Vanity URLs | No DNS, no central namespace. Communities are identified by DHT key. |
| Server-side audit log | No server. Governance entries in SMPL records *are* the distributed, tamper-evident audit trail. |
| Activities / embedded apps | No infrastructure to host iframes. |
| Clyde AI | No AI in the protocol layer. AI lives in client-side plugins, not the message path. |

---

## 14. Design Principles

These principles govern every architectural decision, in order of priority.
When principles conflict, lower-numbered principles win.

1. **No Node Above Another.** Every member is a full peer. No coordinator,
   no special relay, no privileged writer, no leader. The community
   creator's keypair carries no governance authority after genesis writes —
   it is shared infrastructure (required by veilid-core to open records),
   not a credential.

2. **One Equation Everywhere.** The universal SMPL schema (`o_cnt: 0`,
   255 slots) is used for every multi-writer record: governance, registry,
   channels, DMs, group DMs, threads, expression indexes. One write protocol,
   one sync mechanism, one conflict resolution strategy.

3. **DHT Is Primary, Gossip Is Secondary.** SMPL writes are the durable
   source of truth. Gossip is a fast-delivery optimization. If gossip fails,
   Path 3 finds the SMPL record eventually.

4. **Storage IS the Vote.** DHT records expire if not refreshed within
   ~1 hour. Active communities stay alive through normal usage. Dead
   communities decay naturally. No garbage collection, no admin cleanup, no
   storage quotas — the DHT's TTL provides organic lifecycle management.

5. **Privacy Is a Stamina Budget.** Voice uses `Unsafe` for low latency.
   Text uses 1–2 hop safety routes. Sensitive governance uses 2–3 hop routes.
   A slider, not a switch.

6. **Assume Everything Degrades.** Three-path delivery exists because
   gossip, SMPL writes, and DHT watches all individually fail. The sync
   protocol detects and repairs gaps regardless of which path failed.

7. **Reader Validates, Not Writer.** Any member can write any entry. Every
   reader independently checks the writer's permission against the merged
   CRDT state. Invalid entries are silently dropped — they waste storage
   space but cannot affect correctness.

8. **Governance Is Replaceable; Infrastructure Is Not.** SMPL, gossip,
   Veilid routing, and the encryption layers are load-bearing. CRDT merge
   rules, role definitions, ban policies, automod can be changed without
   touching the transport layer. `rekindle-governance` is deliberately
   isolated from I/O for exactly this reason.

9. **Grow Organically, Not by Plan.** Plate Gates split fractal as the
   community grows. Ziplines emerge from gossip reliability metrics. Strand
   Relay topology follows friendship. Nothing is centrally planned.

10. **All Roads Through Veilid.** No external transport. No STUN, no TURN,
    no WebRTC, no raw TCP/UDP, no HTTP (except optional OpenGraph fetch).
    All peer communication goes through `app_message`, `app_call`, and DHT
    operations. By committing fully to Veilid, Rekindle inherits all
    upstream improvements to connectivity, performance, and privacy.

11. **Honest About Tradeoffs.** Section 12 exists because of this principle.
    Every architectural decision that makes Rekindle worse than Discord in
    some dimension is documented with the specific tradeoff. Users should
    never be surprised by a limitation.

12. **Mutual Aid Is the Incentive.** No tokens, no cryptocurrency, no
    payment rails, no premium tiers. Members contribute bandwidth, storage,
    and relay capacity because they benefit from the community's existence.
    Mutual-aid patterns (record warming, history advertising, watch
    relaying) are cooperative behaviors that emerge from self-interest.

---

## 15. Where the Code Lives

| Concern | Location |
|---------|----------|
| Pure CRDT merge engine | `crates/rekindle-governance/` — Tier 6, no I/O, no async |
| SMPL record lifecycle, retry queue | `crates/rekindle-records/` — Tier 3 |
| Gossip primitives (D-fanout, dedup, Lamport) | `crates/rekindle-gossip/` — Tier 5 |
| Private route lifecycle, peer cache | `crates/rekindle-route/` — Tier 4 |
| Signed envelope build/verify, dedup | `crates/rekindle-codec/` — Tier 3 |
| Cross-device sync, gap detection | `crates/rekindle-sync/` |
| Veilid integration + Cap'n Proto schemas (desktop) | `crates/rekindle-protocol/` |
| Unified Veilid boundary (daemon track) | `crates/rekindle-transport/` |
| Voice pipeline | `crates/rekindle-voice/` |
| Lost Cargo file delivery | `crates/rekindle-files/` |
| Direct calls (X25519 call key) | `crates/rekindle-calls/` |
| Video/screen-share fragmentation | `crates/rekindle-video/` |
| DM/group-DM logic | `crates/rekindle-dm/` |
| OpenGraph fetcher | `crates/rekindle-link-preview/` |
| Tauri shell, services, state, IPC | `src-tauri/` |
| Community gossip, governance, presence services | `src-tauri/src/services/community/` |
| Community IPC commands (~127 commands, 30 modules) | `src-tauri/src/commands/community/` |
| Cap'n Proto schemas | `schemas/community.capnp` and friends |

For deeper reference:

- **[`overview.md`](overview.md)** — full system stack, layer
  responsibilities, data-flow diagrams.
- **[`crates.md`](crates.md)** — every crate, its tier, its module layout.
- **[`../protocol/overview.md`](../protocol/overview.md)** — wire formats,
  Veilid primitives, DHT record layouts, MessageEnvelope details.
- **[`../security/overview.md`](../security/overview.md)** — five-layer
  encryption stack, identity system, threat model, MEK + Signal coexistence.
- **[`data-layer.md`](data-layer.md)** — SQLite schema, Stronghold vault,
  DHT record layout in storage terms.
- **[`../roadmap.md`](../roadmap.md)** — phased migration progress.

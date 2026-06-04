# Communities — Governance, Join, Scaling

This document covers the governance internals of Communities:
CRDT merge rules, the pure / async split between
`rekindle-governance` and `rekindle-governance-runtime`,
self-sovereign join, and Plate Gate scaling beyond 255 members.

The companion docs are
[`communities-overview.md`](communities-overview.md) (model,
three-path delivery, SMPL schema, permissions, design principles)
and [`communities-channels.md`](communities-channels.md) (MEK,
channel messaging, voice, DMs).

## 1. CRDT Governance

The merge engine lives in `crates/rekindle-governance/` (Tier 6).
It is **pure logic** — no I/O, no async, no side effects — so it
is deterministically testable and the same merge function runs
identically on every peer.

The async lifecycle on top of it — origin (community creation),
bootstrap (fetch origin + governance + members on join), join,
segments (Plate Gate expansion), apply (writes that flow through
`rekindle_governance::merge::apply`) — lives in
`crates/rekindle-governance-runtime/`. The split exists because
the merge engine must stay free of `tokio` so property tests can
run synchronously over millions of generated inputs without
spawning a runtime per case. See
[`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md)
for the tier-bump argument.

### Merge rules by entry type

Every governance entry includes a `lamport: u64` clock. The merge
sorts all entries from all subkeys by
`(lamport, author_pseudonym)` for deterministic total order, then
applies type-specific rules:

| Entry type | Strategy | Notes |
|------------|----------|-------|
| `ChannelCreated` / `ChannelArchived` | OR-Set | Active = created MINUS archived (matched by `channel_id`). |
| `RoleDefinition` | LWW-Register | Highest `lamport` wins per `role_id`. Ties: lexicographic pseudonym. |
| `RoleAssignment` / `RoleUnassignment` | LWW-Flag | Per `(target_pseudonym, role_id)`. |
| `BanEntry` / `UnbanEntry` | LWW-Flag | Per `target_pseudonym`. Unban requires higher `lamport` than the ban. |
| `MEKGenerationBump` | Max-Register | Highest `generation` is current MEK gen. Reader-validates rotator authority. |
| `CommunityMeta` | LWW-Register | Single logical object — latest write replaces all fields. |
| `ThreadCreated` / `ThreadArchived` | OR-Set | `ThreadArchived` requires `MANAGE_THREADS`. |
| `EventCreated` / `EventArchived` | OR-Set | LWW within active set for metadata. |
| `ExpressionAdded` | OR-Set + tombstone | Removal via `AdminDelete` targeting the entry. |
| `CategoryCreated` / `CategoryArchived` | OR-Set | Same as channels. |
| `PermissionOverwrite` | LWW-Register | Per `(channel_id, target_id)`. |
| `AutoModRule` | LWW-Register | Per `rule_id`. |
| `SegmentAdded` | Grow-Only Set | Plate-Gate segments accumulate. |

For channel records:

| Entry | Strategy | Notes |
|-------|----------|-------|
| `Message` | Grow-Only Set | All messages from all subkeys included; ordered by `(lamport, pseudonym)`. |
| `Edit` | Author-LWW | Edits applied only if the edit's author matches the original message author. |
| `Delete` | Tombstone | Permanent. Removes message from materialised view. |
| `Reaction` | PN-Counter | Distinct pseudonyms with `added: true` at their latest `lamport`. |
| `PollVote` | LWW-Flag | Per `(poll_id, pseudonym)`. |

### The reader-validates principle

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

- A member without `MANAGE_CHANNELS` *can* write `ChannelCreated`.
  Honest clients ignore it.
- A banned member can keep writing to their subkey forever.
  Honest clients drop everything they wrote after the ban entry's
  `lamport`.
- Security comes from *every reader independently validating*, not
  from preventing writes. The SMPL schema gives every member
  irrevocable write access to their own subkey by construction.

### Genesis validation

Entries at Veilid sequence 1 (the first write to a subkey) are
**always accepted** regardless of permissions. This bootstraps
the community: the creator writes initial roles, channels, and
metadata in their genesis entries before any permission structure
exists.

### Circular dependency resolution

Role assignments determine who can make governance changes, but
role assignments are themselves governance changes. Resolution is
by Lamport order: the merge processes entries in chronological
order, and at each entry the *currently accumulated* permission
state determines validity. Genesis entries bypass this check.

```rust
fn merge_governance(subkeys: &[SubkeyPayload]) -> GovernanceState {
    let mut state = GovernanceState::default();
    let mut all_entries: Vec<(PublicKey, &GovernanceEntry)> = Vec::new();

    for payload in subkeys {
        for entry in &payload.entries {
            all_entries.push((payload.pseudonym, entry));
        }
    }
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

The merge function is property-tested in
`rekindle-governance/proptest-regressions/` for convergence (same
entries in any order → same state), idempotence (applying same
entry twice → same state), and commutativity.

### The runtime layer

`rekindle-governance-runtime` wraps the pure merge with the
asynchronous lifecycle: opening DHT records, fetching segment
authors, applying entries to the in-memory snapshot, emitting UI
events. Its modules:

| Module | Role |
|---|---|
| `apply` | The write pipeline: stage entry → merge → persist → emit |
| `origin` | Community creation (genesis entries + first MEK + initial roles / channels) |
| `bootstrap` | Build the BootstrapBundle response for incoming joiners |
| `join` + `join_stages` + `join_gate` | Joiner flow (decrypt invite, scan registry, claim slot, fetch MEK) |
| `dht_hydration` | Rehydrate registry slots from the network |
| `segments` | Plate Gate segmentation (Section 3 below) |
| `roles`, `membership_events`, `overflow`, `invite_secrets`, `event`, `error` | Supporting concerns |
| `deps` | The `Deps` trait surface implemented by `src-tauri/src/services/governance_adapter/` |

The crate itself never imports `veilid-core`, `tauri`, `rusqlite`,
or `iota_stronghold`. The adapter at
`src-tauri/src/services/governance_adapter/` is the
**Schwarzschild boundary** between the runtime crate and the
Tauri-side AppState — see [`services-pattern.md`](services-pattern.md)
for the standard runtime / adapter / pure-logic split.

## 2. Self-Sovereign Join

Joining requires no coordinator approval, no online ceremony
beyond DHT reads and a single slot claim. The joiner has all
cryptographic material in the invite and performs all operations
independently.

### Invite structure

An invite is an encrypted blob distributed out-of-band (deep link,
QR code, peer share). The decryption key is encoded in the deep
link URL fragment (never sent to any server):

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

### Join sequence

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
| 12 | Bootstrap gossip peers | Read every occupied registry slot; pick D online peers as initial neighbours. |
| 13 | Watch all records | Governance, registry, every channel. |
| 14 | Start presence heartbeat | 15 s interval, refreshes route blob. |
| 15 | SMPL catchup | Read all subkeys of all channel records, merge-sort, decrypt, store in SQLite. Background. |

### BootstrapBundle — porter delivery

The bundle is a single `app_call` to the inviter that returns
pre-merged governance entries, online member list with route
blobs, channel MEKs wrapped per-channel via X25519 ECDH, the last
50 messages per channel (MEK-encrypted ciphertext), and the owner
keypair wrapped for the joiner.

Saves ~30 independent DHT reads, reducing join time from
~10–30 s to ~1–2 s. The joiner verifies the bundle against DHT
reads — if anything mismatches, the DHT is authoritative. The
bundle is cargo delivered by a porter; the recipient verifies the
seal. If the inviter is offline, the joiner falls through to
direct DHT reads. The join is self-sovereign regardless of
inviter availability.

### Leave and rejoin

Leaving is unilateral: stop heartbeat, zero own registry slot,
close records, optionally delete local SQLite data. No
coordinator approval. The slot becomes available for reuse.

A leaver who rejoins via a new invite derives the *same*
pseudonym (deterministic from `master_secret` + `community_id`)
but claims a new slot — their previous messages remain attributed
to the same identity.

## 3. Plate-Gate Scaling (Past 255 Members)

A single SMPL DHT record holds 255 member subkeys. Communities
larger than that are split into **fractal segments** — additional
registry + governance records announced via
`GovernanceEntry::SegmentAdded`.

```rust
GovernanceEntry::SegmentAdded {
    segment_index: u16,
    governance_key: TypedKey,
    registry_key: TypedKey,
    slot_range: (u16, u16),  // member slot range this segment covers
    lamport: u64,
}
```

| Community size | Registry segments | Governance segments | Channel segments |
|----------------|-------------------|---------------------|------------------|
| ≤ 255 | 1 | 1 | 1 per channel |
| 256–510 | 2 | 2 | On demand (lazy, deferred to C1-2) |
| 511–765 | 3 | 3 | On demand |
| ~1000 | 4 | 4 | On demand |

### CRDT mechanics

The CRDT model is an **ORMap-of-CRDTs** (Shapiro 2011,
Almeida 2016): each segment is its own join-semilattice; the
community state is the product CRDT under coordinate-wise join.
**Cross-segment invariants are reader-validated, never written
into per-segment state.** Every peer fetches each segment's
author entries and runs the same `rekindle_governance::merge`
over the union.

### What ships today (C1)

| Concern | Mechanism |
|---------|-----------|
| Membership discovery | `services/community/segments.rs::segment_descriptors` lists every active segment from merged governance state. Presence poll iterates all segments. |
| Admin expansion | `expand_community_segment` writes a `SegmentAdded` entry that creates the new SMPL records. |
| Slot claim | `services/community/join/flow.rs` walks segment descriptors in order, claims the first free slot in any segment. |
| Governance fetch | `commands/auth.rs::rebuild_governance_from_dht` does a two-pass merge: primary segment first, then every additional segment. CRDT idempotence makes the second pass safe. |
| Gossip | Crosses segment boundaries naturally — keyed by `(community_id, channel_id)`, not by segment. |
| Hard cap | `MAX_SEGMENTS = 8` (≈2 040 members) — soft cap; raising the constant lifts the limit at the cost of read amplification on presence poll. |

### Channel records and C1-2

Channel records are themselves segmented at scale, but lazy —
created only when the first member of a new segment writes to
that channel. Lazy per-segment channel records ship in **C1-2**
along with cross-segment MEK distribution and the
`ChannelSegmentLinked` governance entry.

Until C1-2 lands, communities that have expanded past one
segment have two behaviours:

1. **Online recipients:** Gossip carries every message regardless
   of segment, so live conversations work everywhere.
2. **Offline recipients in segments ≥ 1:** They will not catch up
   via Path 1 until C1-2 introduces per-segment channel records.

The alternative — bridging via a designated segment-0 relay peer
— would reintroduce a single point of failure of exactly the
kind v2.0 was built to remove. Lazy per-segment channel records
are the spec-mandated route; shipping a relay-bridge stopgap
would make landing the real solution harder.

External references: Shapiro 2011 *Conflict-Free Replicated Data
Types*, Almeida 2016 *Delta State Replicated Data Types*
(arXiv:1603.01529), Riak DT, Matrix faster-joins, Discord guild
sharding.

## Where the code lives

| Concern | Location |
|---|---|
| Pure CRDT merge | `crates/rekindle-governance/` |
| Async lifecycle (origin, bootstrap, join, segments, apply) | `crates/rekindle-governance-runtime/` |
| Governance adapter (Tauri) | `src-tauri/src/services/governance_adapter/` |
| State helpers for governance | `src-tauri/src/state_helpers/{governance, governance_persist}.rs` |
| Governance persistence cache | `governance_entries_cache` SQLite table — see [`data-layer.md`](data-layer.md) |
| Join services | `src-tauri/src/services/community/join/{flow,bootstrap,helpers,history,rejoin,state}.rs` |
| Segment admin command | `src-tauri/src/commands/community/segments.rs` |
| Plate Gate descriptors | `src-tauri/src/services/community/segments.rs` |

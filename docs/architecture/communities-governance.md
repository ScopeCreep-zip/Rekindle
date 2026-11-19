# Communities — Governance, Join, Scaling

This document covers the governance internals of Communities as built
(plan E3.2, ADR 0011): member-sovereign records, the signed governance
op DAG and its merge, checkpoints and the pre-join hint, the
self-sovereign join, the watch ring, and the segment and slot-claim
mechanisms that remain until E3.4.

The companion docs are
[`communities-overview.md`](communities-overview.md) (model,
delivery paths, records, permissions, design principles)
and [`communities-channels.md`](communities-channels.md) (MEK,
channel messaging, voice, DMs).

## 1. Records

| Record | Kind | Owner | Contents |
|---|---|---|---|
| Anchor `A` | DFLT(1) | none: the creation key is discarded after one write | subkey 0 `GenesisCert` (`dht_layout::community_anchor::GENESIS_CERT`) |
| Member record `M` | DFLT(8) | `K_M(p)`, a key only that member holds (`VaultKey::MemberRecordOwner`) | 0 card, 1 presence beat, 2 op-log head, 3 frontier, 4 outbox, 5 history ad, 6 routes (`dht_layout::member_record`) |
| Sealed page | DFLT(1) | discarded after one write | an op-log or channel-outbox page (`dht_layout::sealed_page`) |

`M`/4 holds the head of the member's channel outbox chain, the same N2
chain format as the op log on `M`/2 (`rekindle_records::chain`, never
retired). Each entry is an `OutboxEntry` (`schemas/member_record.capnp`,
`rekindle-codec` `member_record::outbox_entry`): `prev` (BLAKE3 of the
author's previous entry), the channel tag, `streamSeq`, the key
generation, the commit time, the kind and its entry, signed by the
author's pseudonym under `rekindle-outbox-entry-v1` over the anchor and
the entry's chain seq. The writer (`rekindle-channel` `outbox_chain`)
signs a drain at the seqs after the chain's tip, appends it page by page
with `append_exact`, skips entries a retried drain already committed (by
tag and `streamSeq`), and refuses an entry no sealed page holds alone
(`ChannelError::EntryTooLarge`). Bodies are sealed by `outbox_seal` at the
entry's position (see `security/crypto-primitives.md` §3). Posts still go
to the channel SMPL records until the `channel_post` stream's sender and
the `M`/4 readers land (plan E3.3 slices 5–6).

The community id is the anchor's record key. The `GenesisCert`
(`rekindle-codec/src/governance_op/genesis.rs`) names the creator's
pseudonym, the creator's `M` and its owner, and the admission policy;
admission is genesis-only, so no op changes it. Nobody can write `A`
again, and nobody but `p` can write `p`'s `M`: Veilid refuses a write
signed by any other key (`rekindle-netsim/src/member_record_tests.rs`
proves it for both records over the production pool).

## 2. The governance op DAG

The merge engine lives in `crates/rekindle-governance/` (Tier 6). It is
pure logic (no I/O, no async), so the same merge runs on every peer and
is property-tested. The async lifecycle on top of it lives in
`crates/rekindle-governance-runtime/`; the Host's adapter is
`crates/rekindle-node/src/host/adapters/governance/` on both the daemon
and the desktop (ADR 0010). See
[`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md)
for the tier split.

### Ops

Every governance change is a `GovernanceOp`
(`rekindle-types/src/governance/op.rs`, wire schema
`schemas/governance_op.capnp`): author, `seq` in the author's own chain,
`prev`, causal `deps` (`OpRef`s naming the op hash, author, seq and the
author's record), `issued_at` (a tie-break only), the body (a
`GovernanceEntry`) and an Ed25519 signature over
`GOV_OP ‖ anchor ‖ canonical unsigned bytes` (`rekindle-gov-op-v1`; layout in
[`crypto-primitives.md`](../security/crypto-primitives.md)). Ops are appended to the
author's own op log (`M`/2 head, sealed pages behind it) and the
author's frontier (`M`/3: heads plus the roster checksum) is written
after the head. A reader verifies each op
(`rekindle-governance/src/op.rs` `VerifiedOp::verify_with`): signature,
invite and owner proofs, chain, and deps; the op's lamport is derived,
`1 + max(deps)`, never carried. Ops are stored in `governance_ops` and
`governance_heads` (see [`data-layer.md`](data-layer.md)) with a fate
(applied, pending deps, quarantined).

### The merge

`rekindle_governance::merge` folds a verified op set into
`GovernanceState`:

- **The creator** is the `GenesisCert`'s (MSC4289's immutable creator):
  no op at any lamport becomes the creator
  (`merge/op_proptests.rs` `no_op_at_any_lamport_becomes_the_creator`;
  netsim `scenarios.rs` `a_lamport_zero_entry_never_becomes_the_creator`).
- **Power first.** Each op is judged against the power state in its own
  causal past, never the final one; power ops are resolved on a
  mainline first (`merge/resolve.rs`, `power.rs`), with a Discord-style
  hierarchy (a target must rank below the writer).
- **Strong removal.** A kick or ban carries `targetSeqFrontier`, its
  target's highest seq in its causal past, and masks only the target's
  ops above it; a removed manager's concurrent actions are void, and
  the invalidation chains (`op_proptests.rs`
  `removal_invalidates_the_removed_managers_concurrent_actions`,
  `strong_removal_chains_transitively`).
- **Quarantine.** An op the merge refuses after its author's removal, a
  checkpoint equivocation, or fork evidence is quarantined, never
  applied in part, and announced once as `GovernanceOpQuarantined`
  (`rekindle-governance-runtime/src/quarantine.rs`). The signature
  covers the op's Cap'n Proto canonical form without `sig`, which a reader
  computes from the received message itself
  (`rekindle-codec` `governance_op::read_op`), so an op of a kind or with
  fields this version does not know still has a checkable signature. Such
  an op is held (never dropped, never applied) and, when a current member
  signed it, quarantines the community: `GovernanceOpQuarantined` with
  reason `unknownOpKind` or `unknownOpVersion`, shown as a community notice
  (D15). The state is the merge without it, and an op citing it waits.
- **Equivocation.** Ops are signed by per-community device keys the
  pseudonym endorses (ADR 0022). Two ops at one `(device key, seq)` are
  fork evidence: that key is retired, ops below the fork stay valid, the
  conflicting ops and its later ops are quarantined. The anti-entropy
  sweep compares frontiers and finds it (netsim `equivocation.rs`).
- **Convergence check.** `rekindle_governance::checksum::roster_checksum`
  hashes one canonical encoding of every state field; the frontier
  publishes it, and a reader at equal heads compares it
  (`hinted_walk.rs` `ring_holds`). Members holding one op set compute
  equal checksums and tables (netsim `roster_convergence.rs`).

There is no governance clock and no per-entry lamport: the old
`(lamport, author)` sort over SMPL subkeys, "genesis at sequence 1", the
`governance_entries_cache` table and subkey compaction are gone.

### Roles

A role is named by a 16-byte `RoleId` (`rekindle-types/src/id.rs`),
written as 32 lowercase hex characters on IPC, in events and in
`community_roles.role_id`. The Host emits `RolesChanged` and
`EffectivePermissionsChanged` from the merge's state diff, and the
frontend reads the Host's permission flags rather than recomputing them
(netsim `governance_diff_events.rs`).

### Checkpoints and the pre-join hint

A checkpoint (`rekindle-governance/src/checkpoint/`) is an acceleration
hint, never authoritative: a power holder proposes a cut once
`PROPOSAL_OPS` (128) ops lie above the last one
(`checkpoint/endorse.rs`), it commits to the frontier, a state root
(the canonical state followed by the cut index) and the member
directory pages, and a strict majority of the electorate endorses it.
Before its walk a joiner asks its inviter for the usable hint at the
greatest height and the chain behind it
(`rekindle-governance-runtime/src/checkpoint_hint.rs`), resumes from the
hinted snapshot and fetches only the ops above the cut
(`hinted_walk.rs`, `join_walk.rs`), so a join's cost is bounded by the
ops since the latest checkpoint, not by the community's age
(netsim `join_from_hint.rs`, `racing_join.rs`).

### Partial state

A community joined from a hint is **partial** until its backfill ends
(`rekindle-governance-runtime/src/partial.rs`; Synapse's partial-state
rooms): it proposes and endorses no checkpoint, answers no hint,
snapshot or bootstrap request, and compares no roster checksum. The
flag is durable (`governance_partial`). When the backfill ends the full
state at the cut is compared with the hint's digest; a mismatch is a
false snapshot, published as a typed event, and the community stays on
the backfilled state.

### Watching: the ring and the sweep

Members are ordered by `blake3("rekindle-watch-ring-v1" ‖ communityId ‖ p)`
and each watches the `M` of its `RING_K` = 4 ring successors
(`rekindle-governance-runtime/src/watch_ring.rs`), so every `M` has
`min(4, N − 1)` watchers, under Veilid's per-record watcher limit (netsim
`watch_ring_budget.rs`, 40 members). A watched frontier change (`M`/3)
triggers a rebuild (`ring_frontier.rs`); ring members keep each other's
records warm. The anti-entropy sweep (`anti_entropy.rs`) inspects
`SWEEP_SAMPLE` = 16 random members' records every `SWEEP_PERIOD` = 60 s,
and every record on login or reconnect, and fetches any op it lacks.
`governanceOp @74` gossip and `headAdvanced @76` hints are the fast path;
the ring and the sweep are the durable one. There are no governance
record watches and no polling of governance.

## 3. Self-Sovereign Join

The join needs no coordinator and no member online
(netsim `join_alone.rs`: a 30 s and a 24 h absence). The joiner holds an invite link whose code
opens the `InviteSecrets` (`rekindle-governance-runtime/src/invite_secrets.rs`):
the anchor, the inviter's pseudonym and member record, the segment
registry and slot seed (until E3.4), and no community key (D11). An
invite exists as an `InviteCreated` op (`max_uses` in
`1..=MAX_USES_PER_INVITE`), and its link is handed out only after that
op and the frontier are durable on the inviter's `M`.

1. Read `A` and verify the `GenesisCert`; read the inviter's `M`.
2. Ask the inviter for a checkpoint hint and walk the ops above the cut
   (partial until backfilled); with no answer, walk from genesis.
3. Derive the pseudonym, HKDF(`master_secret`, `community_id`), and
   create the joiner's own `M` with a fresh owner key.
4. Build `MemberJoined` (`join_flow.rs`): the record and its owner, an
   owner proof (only `K_M` can bind the record), an invite proof (the
   invite's join key, seeded by `derive_key("rekindle-invite-join-v1", code)`,
   signs anchor ‖ joiner pseudonym ‖ record ‖ `code_hash`; `code_hash` is that
   key's public half, so every reader verifies the proof without the code and
   only a holder of the code makes it, plan E3.2 S10), and shallow
   deps (`join_deps.rs`: the creator's last genesis op, the inviter's
   latest op, and on a re-join the member's own previous op and every op
   removing it). Publish `M`/0 card, `M`/2, `M`/3, then `M`/1 beat.
5. `BootstrapRequest @42` app_calls the inviter's route (from its `M`/6)
   carrying the encoded `MemberJoined`; the inviter verifies, stores and
   merges it and answers `bootstrapResponseV2 @75`
   (`rekindle-governance-runtime/src/bootstrap.rs`, sized to the app_call
   cap), or a NAK. The answer is convenience: the DHT is authoritative.
   The request is not: a member learns of a joiner only by merging its
   `MemberJoined` (gossip, or this request), since the sweep and the ring
   read only records the member already knows. An exchange nobody
   answered (every member away) is a `bootstrap` outbox row
   (`rekindle-node` `outbox/bootstrap.rs`), tried again with backoff
   capped at the sweep's 60 s, reading the inviter's current route from
   its `M`/6 each time, until a member answers; that member's frontier
   then cites the op and every other member's sweep fetches it. A refusal
   withdraws the join; our leave discards the row.
6. The community key arrives from a member (`RequestMEK`, answered by the
   Host's `mek_request.rs` for members of the merged state); a post made
   before it returns `MekMissing`.

Readers admit the joiner by merging its `MemberJoined`; the receive
bound (`op_log.rs` `receive_bounded`) stores a `MemberJoined` from an
unknown author only with an invite proof naming a live invite.

### Leave and rejoin

Leaving writes a `MemberLeft` op; a kick or ban is a removal op by a
member who outranks the target. A member who rejoins derives the same
pseudonym, binds a fresh `M` (owner-proved), and its `MemberJoined`
depends on its own previous op and every op that removed it, so the
merge sees the removal in the re-join's past (`merge/gates.rs`;
netsim `rejoin_after_kick.rs`, `rejoin_after_leave.rs`). A kicked
member's earlier posts stay readable through the remover's recorded
slot heads (`departed_heads`).

## 4. Segments and the slot claim (until E3.4)

Governance no longer lives in SMPL records. Channel records and the
member registry still do, and the registry slot claim still runs: a
joiner claims a slot in a segment registry for its channel writes and
presence (`rekindle-governance-runtime/src/join_stages/`), and the
segment descriptors come from merged `SegmentAdded` ops. E3.4 replaces
the slot claim and the presence poll with `M`/1 beats; the text below
describes these remaining mechanisms.

### Plate-Gate scaling (past 255 members)

A single SMPL DHT record holds 255 member subkeys. Communities
larger than that are split into **fractal segments** — additional
registry and channel records announced via a `SegmentAdded` op.

```rust
GovernanceEntry::SegmentAdded {
    segment_index: u16,
    anchor: TypedKey,
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
| Op types, `GenesisCert`, `Frontier` | `crates/rekindle-types/src/governance/op.rs` |
| Op and member-record codecs | `crates/rekindle-codec/src/governance_op/`, `crates/rekindle-codec/src/member_record/` |
| Pure merge, power, checkpoints, checksum | `crates/rekindle-governance/src/{merge,power.rs,checkpoint,checksum}` |
| Op log, origin, join, hint, walk, ring, sweep, partial | `crates/rekindle-governance-runtime/src/` |
| Host adapter (daemon and desktop) | `crates/rekindle-node/src/host/adapters/governance/` |
| Op store | `governance_ops`, `governance_heads`, `governance_partial` — see [`data-layer.md`](data-layer.md) |
| Record layouts | `crates/rekindle-types/src/dht_layout.rs` |

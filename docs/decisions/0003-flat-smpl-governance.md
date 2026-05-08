# 0003 — Flat SMPL governance replaces the v1.0 coordinator model

- **Status:** Accepted (supersedes the v1.0 rotating-coordinator design)
- **Date:** 2026-03 (research); 2026-04 (decision); active migration
  in 2026-05 onward

## Context and problem statement

The v1.0 community architecture used a "rotating coordinator" pattern:
the community creator owned a DFLT manifest record, and operations
like channel CRUD or ban management were routed through whichever
member currently held the coordinator role. An audit found 15 critical
bugs (split-brain election, unrecoverable state on hand-off, missing
fencing tokens, dead heartbeat code, race conditions) that turned out
to be **symptoms of structural problems**, not bugs to fix:

1. **DFLT records have a single owner keypair** — new coordinators
   *fundamentally cannot* write to the manifest. The model
   contradicts the storage substrate.
2. **DHT consistency is eventual.** Coordinator election based on DHT
   state is split-brain by construction.
3. **Private routes expire.** A coordinator's relay capability is
   ephemeral; rotation cadence and route refresh both fight us.
4. **No production P2P chat system uses dynamic leader election** for
   group state. Session, Quiet, Briar, Matrix P2P, Berty,
   GossipSub — every one of them either uses static authority
   (creator-as-coordinator) or no coordinator at all.

The v1.0 design tried to fix these symptoms with patches; the audit
made clear that the structure itself was wrong.

## Decision drivers

- **No single point of failure.** A community must survive any
  member's permanent disappearance, including the creator's.
- **No coordinator-bottleneck.** Operations that today route through
  one peer should be peer-mesh operations.
- **Match Veilid's actual primitive set.** SMPL records with `o_cnt:
  0` exist precisely for the "every member writes their own subkey"
  pattern.
- **Reader-side enforcement.** With a CRDT, every peer independently
  validates governance entries — there is no privileged write path
  to compromise.

## Considered options

### Option A — Static creator-as-coordinator (Session, Quiet model)

Creator permanently owns the manifest. Creator must be online for
joins and moderation. Admins can be promoted to delegate.

### Option B — Admin pool with deterministic priority (hybrid)

Manifest still creator-owned; promoted admins share a delegation
secret. Deterministic priority order picks the active admin. No
election.

### Option C — Flat SMPL governance with CRDT merge (selected)

Every member writes governance entries to their own SMPL subkey
(`o_cnt: 0`). Every reader merges all subkeys with deterministic CRDT
rules. Permission enforcement is reader-side. No coordinator, no
election, no privileged write path.

### Option D — Fix v1.0 in place

Keep the rotating-coordinator architecture. Fix the 15 audit findings.
Live with the fundamental limit (DFLT can't rotate ownership).

## Decision outcome

**Chose Option C — flat SMPL governance with CRDT merge.**

This is the only option that fully removes the structural single
point of failure. Option A still depends on the creator being online
for joins. Option B introduces shared secrets that are themselves a
load-bearing risk. Option D papers over a contradiction between the
model and the substrate.

The chiral-network mental model from Death Stranding — every node is
a porter, every node is a waystation — fits SMPL with `o_cnt: 0`
exactly. The Schwarzschild principle (creation event collapses behind
a horizon, leaving only the structure) describes what happens to the
creator's keypair: it remains as the community address but carries no
governance authority after genesis writes.

The architectural details — universal SMPL schema, three-path
delivery, reader-validates permissions, deterministic MEK rotator,
plate-gate scaling, mutual-aid infrastructure — are described in
[`../architecture/communities.md`](../architecture/communities.md).

## Consequences

**Positive.**

- **No single point of failure** at any layer of the community state
  machine.
- **No coordinator unavailability** — there is no coordinator to be
  unavailable.
- **No split-brain** — there is no election to split.
- **Self-sovereign join** — new members claim a slot and write
  themselves; no approval loop.
- **Aligned with the substrate.** SMPL `o_cnt: 0` is exactly the
  shape the protocol needs.
- **Composable scaling.** Plate Gates split fractal beyond 255
  members without changing the merge model.

**Negative.**

- **CRDT validation is more expensive on the reader side.** Every
  reader runs the merge over all subkeys; with 255 members each
  emitting governance entries, the merge does real work. We pay this
  cost in exchange for losing the centralised bottleneck.
- **Eventual consistency window.** During gossip propagation,
  different peers may have different merged states. CRDT
  convergence guarantees they reach the same state, but moderation
  actions take 1–5 seconds to propagate. Honest clients
  retroactively filter via merge order.
- **SMPL records contain "junk" entries** from unauthorized writes
  by misbehaving or banned peers. Wastes a small amount of storage
  per invalid entry but has no correctness impact (readers drop
  them).
- **MEK rotation requires a deterministic rotator.** Solved with
  `blake3(departed ‖ self)` lowest-hash selection plus cascading
  fallback if the chosen rotator is offline; implemented in
  [`../architecture/communities.md` §5](../architecture/communities.md#5-mek-lifecycle-peer-to-peer-no-vault).

**Migration scope.**

- Delete `services/coordinator/` (3,793 lines).
- Replace v1.0 DFLT manifest record with v2.0 SMPL governance record.
- Replace coordinator-mediated join flow with self-sovereign join.
- Replace coordinator-distributed MEK with peer-to-peer rotator.
- Implement `rekindle-governance` (Tier 6, pure CRDT merge).
- Tracked in [`../roadmap.md`](../roadmap.md).

## Pros and cons of the options

### Static creator-as-coordinator

- **+** Mental model is simple.
- **+** Matches every production P2P chat system today.
- **−** Creator must be online for joins.
- **−** Single key-loss event freezes the community forever.

### Admin pool with deterministic priority

- **+** Multiple potential coordinators without election.
- **+** Better availability than Option A.
- **−** Shared delegation secret is a load-bearing risk.
- **−** Still has a "must be online" leader at any moment.

### Flat SMPL CRDT (chosen)

- **+** No single point of failure.
- **+** No election, no leader, no coordination state.
- **+** Self-healing — communities survive any departure.
- **+** Aligned with Veilid's SMPL primitive.
- **−** CRDT merge cost on every reader.
- **−** 1–5 second consistency window during gossip propagation.
- **−** "Junk" entries waste storage.

### Fix v1.0 in place

- **+** Lowest immediate effort.
- **−** Does not address the structural problem.
- **−** DFLT records cannot rotate owner; rotating coordinator is
  contradicted by the substrate.

## More information

- [`../architecture/communities.md`](../architecture/communities.md) — full v2.0 chiral-network architecture
- [`../roadmap.md`](../roadmap.md) — migration phases
- The pre-decision research doc (`coordinator-redesign-research.md`)
  is no longer in the repo; its analysis lives in the v2.0 spec
  itself.

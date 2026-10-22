# 0018 — MEK generation bumps are agreed by write-once ballots and majority confirmation

- **Status:** Accepted (owner ruling O5, 2026-10-15). **Amended 2026-10-22** (plan §0, Q7 decided
  from research): the ballot covers every MEK generation bump of either scope, not only the rotation
  after a removal. The original title was "MEK rotation after a removal is agreed by write-once
  ballots and majority confirmation".
- **Date:** 2026-10-15
- **Amends:** [0012](0012-community-keys-stay-on-mek.md), its "rotation by the deterministic rotator" clause.
  The rest of 0012 stands: per-channel MEKs, signed wraps checked against rank, rotators limited to
  KICK/BAN/MANAGE_COMMUNITY holders, `generation == current + 1`, durable `MemberRemoved`, no MLS.
- **Plan:** D20 (v2.1, 2026-10-05) and step E3.5 in `.claude/plans/standards-remediation/00-integration-plan.md`
  (v2.2). Verified by the C9 partition scenario.
- **Related:** [0011](0011-member-sovereign-records.md) (member records hold the ballot reference),
  [0016](0016-sender-owned-call-media-keys.md) (call media is not under a MEK, so none of this applies to it).

## Context and problem statement

0012 elects the rotator deterministically (today `blake3(departed ‖ self)`, lowest hash wins:
`crates/rekindle-mek-rotation/src/lib.rs:8`, `rotate.rs:63`). Members decide independently, on whatever
membership view they have. During a partition, or when two removals overlap, two rotators each mint a key for
the same generation and the community splits: members hold different keys under one generation and cannot read
each other. The engine audit traced 16 split-brain fix commits to this (V23;
`.claude/plans/architecture-audit/history/h1-commit-chronology.md` D5). The B5 recovery mint and the post-hoc
same-generation tiebreak are patches over the same race. A rotation is irreversible once members encrypt under
it, so it needs agreement before use, not repair after.

0002 and 0012 rejected MLS because it assumes a server's total order (DCGKA, ACM CCS 2021). The agreement here
has to work with no server, on Veilid records, with members offline.

## Decision outcome

**Keep the rotator model; add agreement before the new key is used.**

1. **Write-once ballot.** Every MEK generation bump of either scope (channel or community) is a ballot: after a
   removal, a manual rotation, the scheduled 7-day rotation, an electorate refresh, a D16 history-visibility
   change, and generation 0. *(Amended 2026-10-22, Q7; the original read "each rotation after a removal".)*
   Admission never bumps: an approved joiner gets the current generation's wraps (0011, Q6). The ballot is a
   sealed DFLT(1) record whose owner key is discarded after one write (D15's sealed-page primitive). It cannot
   be rewritten, so it cannot equivocate, and it needs no unreleased Veilid feature (no `min_seqnum`, V22). The
   ballot is referenced from the rotator's own `MemberRecord` (ADR 0011). Nothing mints a generation directly:
   IPC `MekRotate` starts a ballot round, and the 7-day timer only nominates.
2. **Steer-proof winner.** Among valid ballots for one transition the winner is the lowest
   `H(previous epoch content ‖ member)`. The inputs are fixed before anyone votes, so no member can grind for a
   win, and no clock or arrival order is involved (two readers would otherwise order the same ballots
   differently).
3. **Strict-majority confirmation.** The new key is used only after a strict majority of the eligible rotators
   for the previous epoch confirm it. Any two strict majorities of one fixed electorate share a member, so two
   different keys cannot both be confirmed for one generation (the quorum-intersection property that Paxos and
   Raft rest on). If no majority is reachable the rotation waits, as 0012 already requires when no eligible
   rotator is online.
4. **Collision healing.** If two keys for one generation are ever seen, any eligible peer heals with
   generation + 1 wrapped to the union of remaining members (Session's rekey-collision rule,
   `.claude/plans/architecture-audit/research/r3-signal-session.md` R3-E5-4).
5. **Leavers never mint.** A departing or removed member never authors its own departure and never mints its
   successor key.
6. **Announce → ready → execute.** Members switch to the new key in three steps, as DAVE sequences its
   transitions (`dave_protocol.md:93-97`, saved in
   `.claude/plans/standards-remediation/evidence/sources/realignment/media-keys/`), with the ballot and
   confirmations taking the place of DAVE's gateway.
7. **Pure function of the op set.** Every rotation side effect is derived from the converged governance op set,
   so all honest members reach the same result.

The selection-and-confirmation shape was first seen in VeilidChat (`dht_roster_arbitration.dart`). Under the
owner's rule (O9, 2026-10-15) that is a lead, not a precedent; the reasons above stand on their own.

## Consequences

- Deleted at E3.5: the B5 recovery mint ("last-resort mint", `community/mek_rotation.rs`) and the post-hoc
  same-generation tiebreak, because divergence can no longer be produced.
- *(Added 2026-10-22, Q7.)* That deletion is sound only if no generation bump bypasses the ballot. RFC 9420 §14
  rests on the same rule ("only one Commit is processed for each epoch"), and Keybase orders PTK rotations as
  `team.rotate` links in one chain. So E3.5 also deletes the direct-mint paths: `rotate_mek_on_request`
  (`rotate.rs:138`) and the desktop `rotate_mek_local` (`community_mek_local_rotate.rs:23`).
- A kick or ban takes effect for new messages only once a majority confirms. While it waits, the kicked
  member may still read new text under the old key. U1's review for `ban_member` says so ("a key rotation
  starts; it completes when a majority confirms").
- A community with too few eligible rotators online cannot rotate until they return. That is a visible
  pending state, not a fallback.
- New wire and DHT objects: the `mek_ballot` record and `MekBallotRef` (plan §6 ledgers, E3.5).
- CLAUDE.md and the roadmap describe `blake3(departed ‖ self)` as current code until E3.5.

## More information

- [0012](0012-community-keys-stay-on-mek.md), [0002](0002-signal-protocol-for-1to1.md)
- L. Lamport, [Paxos Made Simple](https://lamport.azurewebsites.net/pubs/paxos-simple.pdf) (2001), majority quorums
- [DAVE protocol](https://daveprotocol.com/)
- [RFC 9420](https://www.rfc-editor.org/rfc/rfc9420) §14, one Commit per epoch
- Plan §0 decision log, 2026-10-22 row "Q7 decided"

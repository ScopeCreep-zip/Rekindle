# 0022 — Governance ops are signed by per-community device keys the community pseudonym endorses

- **Status:** Accepted 2026-11-21 (plan E3.2 S7d-keys is built against it). Proposed (decided 2026-10-24 from the research, register Q7 "restored backups"; the owner asked
  to "refer to our plan or see how similar apps behave" rather than be asked). Revised 2026-10-24 after the
  review of the first draft (the record owner, the endorser, the endorsement set, per-device records, the
  re-admission rank rule, the cap and rate limit, the signed layouts and the unlock barrier). Revised again
  2026-10-24 after a second review of the rewrite: effectiveness judged at each op's causal deps, endorsements that sign
  the set they extend, one fork per member in place of the per-interval rate limit, the barrier over every
  answering ring peer, the pseudonym-signed successor's conflict rule, and the card version rule. Corrected
  2026-10-24 (prod research batch 7): §0, what device keys protect, and the cap of 6 in total (rule 6).
  Corrected 2026-10-24 after a review of the companion docs, still Proposed: a member's own effective device key
  may co-sign a new key after a fork (rule 7), a device checks for its own retirement and ops above a `lastSeq`
  are quarantined and shown, not dropped (rules 4 and 5), cards and beats are judged on the reader's converged
  set (rule 3), known ring peers that do not answer leave writes pending (rule 4), the MSC4153/MSC4312 citations
  (rule 0) and Signal's pinned cap line (rule 6). Amended 2026-10-24 (prod research batch 10), still Proposed:
  a device detects a copy of its data root itself, a `DeviceRetired` retires the key for every copy and each
  re-enrols with every copied per-device secret replaced (rule 4), readers never prefer the last writer (rule 5),
  forks count per device key, only pre-fork keys co-sign an own-key `DeviceReadmit`, at most one per checkpoint
  interval and shown to the member's devices and `KICK_MEMBERS` holders, and retiring a key does not retire the
  keys it co-signed (rule 7). Corrected 2026-10-24 (premise check of plan S7c/S7d-keys), still Proposed: the device
  key lives in a separate device vault that identity export never reads, the barrier's timeout bounds one round and a
  restore retires at the barrier's maximum, each device watches its own `M(p,d)`/2, the re-key waits for a
  TrustChanging intent, a `DeviceRetired` is never a second fork, retirement verdicts move only from applied to
  quarantined, the power rows are S3's `power.rs`, E5.3's device root key is one design with these keys, and the
  ADR 0011 passages amended are listed. Corrected 2026-10-24 (pass 2 of that check), still Proposed: a
  `DeviceRetired` whose `lastSeq` is below its carrier's causal past is invalid, quarantined ops stay as DAG ancestry
  and an op that loses its authority with them is quarantined and shown (rule 5), a new key after a fork needs the
  `DeviceReadmit` in its op's deps (rule 7), a restore re-keys only through the `ReEnrolDevice` intent (rule 4), and
  the authority rows are `validate/mod.rs` `permitted` checks, not `power.rs`. Corrected 2026-10-24 (pass 3 of that
  check), still Proposed: plan S7c's checkpoints are acceleration hints that decide no op's fate, so the readmit interval
  counts usable checkpoint heights in an op's deps (rule 7); the `lastSeq` floor is taken over the converged set and an
  op another key builds on is never retired away, its late arrival being the one reverse move, shown (rule 5); a
  refusal caused by a power op later quarantined stays until re-sent, and the quarantine cause is the lowest
  `(lamport, op_hash)` op in the authority path (rule 5); device ops resolve in their own phase, first, and are never
  quarantined by their subject's retirement, and a key whose endorsement has not arrived is pending (rule 6).
  *Decided from research 2026-10-24 (batch 14):* still Proposed: an op whose authority came from a quarantined op is re-judged at its own causal
  position, the power state in its causal past (plan E3.2 S7c.3a), never on the converged set, and a refusal "while a
  power op stood" means that power op is in the refused op's causal past or is a removal whose `targetSeqFrontier`
  masks it (rule 5). Amended 2026-11-22 (lead decision 63, plan K5j), Accepted: after a re-join the
  member's new record has the re-joining device as its first device (rules 3, 10); the person's other devices are
  **not** carried over or re-endorsed (rules 1 and 10 bind every endorsement to the member record; an automatic
  re-endorsement would propagate trust from an ended membership): each re-enrols with a **fresh** device key (rule 2
  revives only retired keys, and a fresh key keeps the incarnations unlinkable) whose `extendsSet` is the new
  record's set, as Signal's re-registration unlinks linked devices, Keybase's reset revokes every device and Matrix's
  cross-signing reset re-verifies each. A device learns its endorsement no longer binds the current member record
  at its unlock catch-up (rule 4), refuses its own writes (`DeviceUnendorsed`), drops its stale `M(p,d)` binding,
  and the Host exposes that state so the frontend offers the device's re-enrolment. A leave or kick and re-join
  starts a new member record, not a fork of the old chain, so it does not count toward rule 7's one fork.
- **Date:** 2026-10-24
- **Amends:** [0011](0011-member-sovereign-records.md) and plan D15, their rule that the member's pseudonym signs
  its own op log. The record owner does not change: `M(p)` stays owned by the random key `K_M(p)` that only the
  member holds (0011 :98-99; R5 rejected a pseudonym-owned record). Only op signing moves, from the pseudonym to
  per-community device keys that the pseudonym endorses. D15's equivocation bullet becomes "two conflicting ops at
  one `(device key, seq)` retire that device key; ops below the fork stay valid". It also fills in 0011's
  "Multiple devices" rule (:152, one `M(p,d)` per device) and replaces its "Key loss or compromise" rule (:150).
  It further amends 0011's Binding rule (:116-118: readers check the device key that signs a record's data, not an
  inner pseudonym signature), its successor rule (:92-93: two conflicting pseudonym-signed successors void each other
  and neither is offered, rule 8) and its watcher bound (:128: each device also watches its own `M(p,d)`/2, so ≤ 5
  watchers per record, rule 4) (corrected 2026-10-24, premise check).
- **Plan:** E3.2 slice S7d-keys (after S7c) in `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2);
  the decision row is in `.claude/plans/standards-remediation/audit-2026-10-23/00-register.md`, "Decided 2026-10-24
  from the research", item Q7.
- **Related:** [0011](0011-member-sovereign-records.md) (member records, `CommunitySuccessor`),
  [0018](0018-mek-rotation-ballots.md) (ballot votes are ops, so they are signed the same way; the electorate rule),
  [0019](0019-intentional-actions.md) (re-admission is a user action).

## Context and problem statement

D15 quarantines an author forever when two different ops carry one `(author, seq)`. The author today is the
community pseudonym `p`, which is derived from the master secret (threat model Z2, `derive_community_pseudonym`),
so the same key exists on every device of the user and after every restore. An honest member who restores an
older backup, an OS snapshot or a copied data root (threat model I14 names these), or who runs the identity on a
second device, resumes at a seq it has already used and signs a different op there. Nothing cryptographic tells
that apart from a malicious equivocation: both are one key signing two ops at one position, and any later
statement the key signs ("I restored", "same person") can be forged by whoever holds the key. Today an honest
restore would therefore mute the person in that community for good.

The research (batch 3, Q7, primary sources at pinned commits) found that no studied system mutes a *user* for
this, and that every one either designs reuse out or recovers through a new key approved by someone else:

- **Signal** never backs up session, prekey or sender-key state or the registrationId (`FullBackupExporter`
  L90-110, `AccountValues` L130-141); re-registration gets a fresh registrationId and peers rebuild sessions
  (HTTP 410, `SignalServiceMessageSender` L3059-3077).
- **Matrix** gives a restored or new login a **new device_id with new keys**, signed by the cross-signing key;
  matrix-rust-sdk permanently rejects new keys under a known device_id (`SigningKeyChanged`), one key binding,
  not the user.
- **Keybase** resets start a new subchain under a new eldest key, which the old key does not sign
  (`reset.avdl` L22-47); a reset team member must be re-added by another admin with a strictly greater
  `EldestSeqno` (`go/teams/chain.go` L1194-1270).
- **Hypercore** freezes a core forever on two signatures at one length (`core.js` L620-703); **Autobase**
  recovers with a new local writer key (`_rotateLocalWriter`, `addWriter`).
- **Certificate Transparency** retires a log whose key signed conflicting tree heads, yet SCTs issued before
  the retirement timestamp still count (RFC 9162 §8, §11.3; Chrome `log_states.md` L209-219).
- **Scuttlebutt** treats a fork as fatal for the feed (`ssb-validate` L143-160) and uses one feed per device.
- **SimpleX** detects replay, fork and gap with message id plus previous hash, and asks the user to confirm a
  repair that names both causes (an old backup or a compromised connection).

Two constraints come from Rekindle itself. The endorser must be the **community pseudonym**, never the global
identity key: one global signer on every community's endorsements would link the user across communities
(threat model L1 "no shared identifier between communities", I8, and the R-section note that a wider signing
chain "would create cross-community linkability"). And the endorser's own statements must not form a
seq-positioned log, because `p` exists on every device and after every restore, so such a log could fork exactly
as today's does.

## Decision outcome

0. **What device keys protect, and what they do not** (*corrected 2026-10-24, prod research batch 7*). Pairing gives
   every device the master secret, and `p` is derived from it, so **a device holding the master secret is the
   user**: a stolen device can endorse its own keys and retire the real ones, and no reader can tell it from the
   owner. Device keys exist so that an honest restore or a second device does not trip D15; they do not stop
   theft. The stolen-device path is an **identity reset**: a new master secret and new pseudonyms, with contacts
   and communities re-established under it (a device that is not cross-signed is not trusted, Matrix MSC4153;
   recovery is a reset of the cross-signing identity, which needs extra authentication and which peers see and
   acknowledge as an identity change, Matrix MSC4312). There is **no existing-device requirement** (Keybase's
   model, where only a device already in the chain can add one): it would break restoring a lost sole device
   from the seed, which must stay possible with nothing but the master secret. An existing device's co-sign is
   one of rule 7's two paths after a fork, never the only one.
1. **Per-community device keys, endorsed by `p`.** Each device holds, per community, a random Ed25519 device key `d`
   minted on that device and never derived from the master secret, so it is unrelated across communities and across
   devices. `p` never signs a governance op. It signs only **endorsements**: `DeviceKey{deviceKey, incarnation,
   extendsSet, deviceRecord, deviceRecordOwner, forkEvidence}` and `DeviceRetired{deviceKey, incarnation, lastSeq}`.
   Every governance op is signed by a device key (`GovernanceOp.author` stays `p`, a new field names `d`). Readers
   accept an op only when its device key is endorsed by `p` for this community and this member record, is effective
   (rules 6 and 7), and is not retired at the op's seq.
2. **The endorsement set is a grow-only set, not a log.** Endorsements and retirements carry no seq and no `prev`.
   Each member's set is keyed by `(device_key, incarnation)` and merges by union, a CRDT, so two devices or a
   restored device adding endorsements never conflict. Each endorsement signs its own context (rule 10): `community
   ‖ memberRecord ‖ deviceKey ‖ incarnation ‖ extendsSet`, where `extendsSet` is the hash of the member's
   endorsement set as the signer held it (the element ids it commits to travel with the endorsement). So an
   element's validity depends only on `p`'s signature, that context and the rules below, and is the same in every op
   that carries it, in whatever order it arrived; readers harvest elements from any op that carries them,
   quarantined ops included, because a device key cannot forge `p`'s signature. An endorsement whose extended set
   the reader does not yet fully hold is pending, not invalid. **Retirement is permanent:** once the set holds a
   valid `DeviceRetired` for `d`, or the op set holds fork evidence for `d` (rule 5), `d` is retired forever,
   whatever the causal order, and no later endorsement of `d` under any incarnation revives it. Incarnations are per
   member; an endorsement's incarnation must exceed every incarnation in the set it extends (Keybase's strictly
   greater `EldestSeqno`). Equal incarnations on distinct keys from concurrent devices are both kept.
3. **One record per device.** Each device key has its own `M(p,d)`, a DFLT(8) record with ADR 0011's subkey map,
   owned by a random key `K_M(p,d)` that only that device holds, so two devices never race on one subkey (0011
   :152). The first device's record is `M(p)`, bound by `MemberJoined` with owner `K_M(p)` as today; every further
   device record is bound by its `DeviceKey` endorsement (`deviceRecord`, `deviceRecordOwner`). Per subkey:
   2 `GovHead` and 3 `Frontier` are that device's own op chain and view; 4 outbox and 5 history ad carry that
   device's posts and advertisements; 1 beat is that device's beat, and the member is online when any effective
   device's beat is live; 0 card is published by each device, which sets its version to 1 + the greatest version
   among the member's effective devices' cards, read before writing (never a fixed or maximal value). Readers show
   the card with the greatest version, ties broken by the lowest device key bytes, and ignore a card whose version
   exceeds 1 + the greatest version among the member's other effective devices' cards, when it has any, so no one
   device pins the card. Readers ignore the card, beat, outbox entries (`M(p,d)`/4, E3.3), history ad and posts of
   a retired or inert key; a retired key's posts above its `lastSeq` never count. Cards (`M`/0) and beats (`M`/1)
   are display state, not ops: a reader judges their device key as effective, inert or retired against its own
   converged endorsement set, not at a causal cut, and governance effects never read them. A device's first op
   (seq 0 in its chain) is the op that carries its own `DeviceKey` endorsement.
4. **Never backed up; catch up at every unlock.** The device key, `K_M(p,d)` and the device's seq head live in the
   vault under `DeviceOpKey{community}` and are excluded from every Rekindle backup and export: they live in a separate device vault, whose key the identity
   vault holds and whose file identity export never reads (export copies the identity vault file whole,
   `rekindle-vault` `export_files`), so an imported identity enrols new device keys (corrected 2026-10-24, premise
   check). That cannot stop a
   whole-directory restore (Time Machine, a btrfs or VM snapshot, a copied data root; I14; E7.1's exclusion is
   best-effort), so the **unlock barrier** is the real defence and is mandatory. At every unlock, before the first
   governance write in each community, the device:
   - reads its own `GovHead` from `M(p,d)`/2 on the DHT (a network read, not the local cache);
   - asks every ring peer (k = 4) it knows for its frontier and, for each peer that answers within one app-call
     timeout (the bound of one round; a round with no answer is retried, rule 4's pending state), walks from that frontier's tips (frontiers hold DAG tips, not per-author heads) to the highest seq
     of `d` reachable from them, the same walk the DAG fetch uses; it takes the **maximum over every answering
     peer**, since a stale or withholding peer can only report lower, never forge `d`'s signature. This covers
     an op that reached peers by gossip before its append landed;
   - when the member knows **no** ring peer at all (an empty ring, as for the sole creator of a new community),
     the barrier completes on its own `GovHead` read from the DHT alone. Ring peers that are known but do not
     answer within one app-call timeout are not an empty ring: governance writes stay pending under the visible
     state below until at least one known ring peer answers, so a fork does not reopen when every ring peer is
     offline;
   - signs nothing while a read is outstanding or a result is above the local head. Writes wait in the
     outbox under a visible "pending: catching up" state; offline, they queue and are never signed at a guessed
     seq.
   If the published head is above the local one, this device was restored: it never signs under `d` again and
   raises the backend-originated TrustChanging `ReEnrolDevice` intent (plan S7d-keys.10; *corrected 2026-10-24,
   pass 2*: this paragraph re-keyed automatically, which rule 9 forbids); when the user commits it, `p`
   signs `DeviceRetired{d, incarnation, lastSeq}` with `lastSeq` the barrier's maximum (the DHT `GovHead` and every
   answering peer's walk, so an op gossiped before its append landed stays valid; corrected 2026-10-24, premise check,
   from "the published head"), and the device mints a fresh device key and
   record with a new endorsement. No fork happened, so this first restore needs no co-signer (*amended 2026-11-23,
   plan E3.2 K4b.4 R3:* the replacement is carried by its own `DeviceReEnrol` op at the new key's seq 0, rule 7;
   until plan E3.2 S7d-keys.10 builds it, the replacement needs a co-signer like any other post-fork key), but it counts as
   the member's one fork under rule 7 (any `DeviceRetired` does, since a reader cannot tell a restore from a
   lost-device retirement: both are `p`-signed): a later restore, fork or new device of that member needs a
   co-signer. A device also signs seq n + 1 only after seq n's append to `M(p,d)`/2 has landed, so no
   gossip-only op is ever the head.
   **A copied data root** leaves two devices running under the same `d` (*amended 2026-10-24, prod research batch
   10*):
   - **Self-detection.** Each device keeps its local signed log of the ops it wrote under `d` and checks against
     it every op of `d` it ingests (gossip, the DAG fetch, the barrier's frontier walk) and every `ValueChange` of
     its own `M(p,d)`/2, which each device watches for this in every community (the ring watches only successors). An op signed by `d` that it did not write (a seq or hash it holds no record of) is a
     clone signal: the device stops signing under `d` and has `p` sign `DeviceRetired{d, incarnation, lastSeq =
     its own local head}` itself, without waiting for the other copy (corrected 2026-10-24, premise check: the
     retirement and its replacement are signed, as one op, when the user commits a backend-originated TrustChanging
     `ReEnrolDevice` intent, ADR 0019, since rule 9 allows no automatic recovery; `lastSeq` is fixed at detection, so
     the wait changes no verdict) (the evidence every clone detector studied
     keys on: Matrix's duplicate one-time key, Signal's 4409, Tailscale Tailnet Lock's rotation tie). This feeds
     the check below.
   - **The check before signing.** Every device checks the endorsement set for a `DeviceRetired` of its own `d`
     before each sign and on every `ValueChange` of its member set. On finding one it stops signing under `d` and
     shows the visible state "this device's key was retired by another copy of this data".
   - **Retired for every holder.** A `DeviceRetired` retires `d` for every copy; no copy keeps it, so the copies
     cannot displace each other (Signal's 4409 ping-pong, Tailscale DERP's last-writer flapping), and **each**
     copy re-enrols with a fresh key, its co-signer per rule 7. The whole data root was copied, so a clone signal
     in one community retires the device's key in every community it holds.
   - **Every copied per-device secret is replaced**, not only the op key (Tailscale's `--force-reauth` leaves the
     shared machine key): the per-community device keys and their `K_M(p,d)` records, the device's DM and ratchet
     state (its E2.4 prekey record and ratchet sessions, re-established through the prompt-first path, never
     silently) and its sync device state (the E5.3 device root key, `identity.device_id` and its DeviceList
     entry). The master secret and the pseudonyms stay shared by design (rule 0), so this is not theft recovery.
   - **The race is stated:** ops either copy signed above `lastSeq` are quarantined and shown (rule 5), not lost,
     and their author re-sends them under the new key.
   - Wrapping the device op key in the OS keystore, non-exportable where the platform allows, would make a copied
     data root fail closed at start (Tailscale `--encrypt-state`); it is a platform lead for plan E5.3, with its
     verification step there, not a rule here. The same-machine instance lock stays as a separate guard against
     two processes on one data root and is not the defence (matrix-rust-sdk's cross-process store lock and
     Element's `SessionLock` cover one store only).
5. **A fork retires the device key, not the person.** Two different ops at one `(device key, seq)` are fork
   evidence, and its hash is `BLAKE3 derive_key("rekindle-gov-fork-evidence-v1")` over `community ‖ device key ‖
   seq ‖` the two op hashes in byte order. The evidence retires that device key. Ops of that key below the fork seq
   stay valid and stay in the converged state (CT keeps pre-retirement SCTs); both conflicting ops and every later
   op of that key are quarantined. An op above a `DeviceRetired`'s `lastSeq` is not in the converged state and never counted as
   an equivocation (one already applied moves to quarantined, shown, rule 6; *corrected 2026-10-24, pass 2*, from
   "is never applied"); it is quarantined with a visible reason and never dropped silently (plan D15's quarantine
   rule), so a copied data root's race (rule 4) stays visible. Readers never prefer the last writer of `d`
   (*amended 2026-10-24, prod research batch 10*): when the set holds more than one `DeviceRetired` for one key,
   the lowest *effective* `lastSeq` over every `DeviceRetired` and every carrier of it counts (*amended 2026-11-23,
   Reading A, plan E3.2 K4b Q2:* it read "the lowest `lastSeq` counts"; one `DeviceRetired` carried by several ops
   keeps the highest of its carriers' floors, so a later carrier that saw less never lowers it, and a carrier of a
   `DeviceRetired` of the key is not a witness of it), and a `DeviceRetired` retires the key only above its effective `lastSeq`, the highest of its signed
   `lastSeq`, the key's highest seq in the causal past of any op of another key in the converged set, and the key's
   highest seq in its carrier's causal past (*Corrected 2026-10-24 (pass 3):* it read "a `DeviceRetired` whose `lastSeq` is below the key's
   highest seq in its carrier's causal past is invalid and quarantined", which a carrier with genesis-only deps
   bypassed; the key's own ops are left out, or a copy's own chain would lift the floor over everything it wrote), so
   nobody erases ops others built on; when such an op arrives after a reader quarantined the ops it builds on, exactly
   those move back to applied, shown, the one reverse move, since a reader holding the whole set applies them; two different ops of `d` at one seq at or below it are shown as an equivocation
   pair (fork evidence, above), and every op of `d` above it, from either copy, is quarantined and shown (Tailnet
   Lock rejects both clones on a tie). The author's re-enrolled device offers "re-send under new key" for its own
   quarantined ops; nothing inbound is lost, since records are not consumed on read. A replay of the same op is idempotent and never counts.
   Quarantined ops stay in the DAG as ancestry and are never applied, so the ops that cite them resolve and are
   never pending on their account; an op whose authority came from a quarantined op is re-judged at its own causal
   position, against the power state in its causal past with the quarantined op left out (*Decided from research 2026-10-24 (batch 14):* it read "on the
   converged set"; plan E3.2 S7c.3a), and moves to quarantined, shown with that cause, only if its authority fails without it (*added
   2026-10-24, pass 2*); with several, the cause is the lowest `(lamport, op_hash)` quarantined op in its authority
   path. The move is one-way for refusals (*Corrected 2026-10-24 (pass 3)*): an op refused while a power op stood (in its causal past, or a removal whose
   `targetSeqFrontier` masks it, S7c.3a; batch 14), a ban's or kick's among them, stays refused after that power op is quarantined unless its own causal past holds the quarantine's
   cause, so its author re-sends it; nothing members saw refused is revived without its author acting, and the rule
   is a function of the op set, so every reader agrees. The person is not muted:
   their other effective device keys keep working (until a second fork, rule 7), and they are shown as having a
   forked device (rule 9).
6. **Device cap, judged at each op's causal deps.** A member has at most **6** effective device keys in total,
   the first device included, Signal's count (`MAX_DEVICES` is 6, checked as `getDevices().size() >= MAX_DEVICES`:
   the primary plus 5 linked; Signal-Server `AccountsManager.java` L204 at commit `8edf3488`; *corrected
   2026-10-24, prod research batch 7*, from 5, which counted one too few; plan E5.3 uses the same number). With no
   server, readers enforce it. Effectiveness, the
   cap included, is judged for each op at that op's causal deps, as moderator authority is (E3.3 M1), never on
   the converged set: a key is effective for op `o` if its endorsement is in `o`'s causal past (or carried by `o`
   itself at seq 0), it is among the first 6 live endorsements by `(incarnation, device key bytes)` in the
   member's set at `o`'s deps, and rule 7 admits it there. Once judged, effectiveness never changes (retirement and fork evidence are judged on the converged set and can
   only move an op from applied to quarantined, shown, never back, but for rule 5's floor; corrected 2026-10-24, premise check; *Corrected 2026-10-24 (pass 3)*), so a concurrent
   endorsement that converges later cannot void ops already accepted. An endorsement whose own extended set
   already holds 6 live keys is inert in every carrier and can never be re-carried into effect. All of this is
   a pure function of the op set (D15 "pure effects"), never of local time. Device ops resolve in their own
   phase, before power ops, in `(lamport, op_hash)` order (*amended 2026-11-23, plan E3.2 K4b Q6:* the device phase
   orders the device view only; a moderator's `DeviceReadmit` is judged at its causal position, its `KICK_MEMBERS`
   and outranking check against the power state in its past, the kick rule of E3.3 M1 / S7c.3a, never at genesis;
   and a `DeviceReadmit` enters the device view only once applied, so one its author has no authority for never
   makes its key effective, K4b security finding); a device op is quarantined only through fork evidence or a
   retirement of the key that signed it, never of its subject, so a `DeviceReadmit` survives its subject's retirement.
   An op of a key whose endorsement has not arrived is pending, until its endorsement arrives, never quarantined
   (*Corrected 2026-10-24 (pass 3)*).
7. **One fork per member.** After a member's first fork (fork evidence for any of its device keys, or any
   `DeviceRetired`, rule 4) in an op's deps, every new endorsement for that member becomes effective only together
   with a `DeviceReadmit{member, memberRecord, newKey, incarnation, forkEvidence}` op that names it in the
   deps of the op being judged (*corrected 2026-10-24, pass 2*, from "a converged": effectiveness is judged at
   deps, rule 6),
   co-signed by **either** a `KICK_MEMBERS` holder who outranks the member (rule 8) **or** one of the member's own
   device keys that is effective at that `DeviceReadmit`'s causal deps and was endorsed before the member's first
   fork (Signal's precedent: an existing device approves a link). Forks are counted per device key: fork evidence
   and any number of `DeviceRetired`s for one key, such as both copies of a copied data root retiring it (rule 4),
   are one fork. The own-key path gives creators and top-rank members, whom nobody outranks, a way to add a device
   while they still hold a pre-fork key. It is bounded (*amended 2026-10-24, prod research batch 10*):
   - **Pre-fork keys only.** The co-signer's endorsement must be in the causal past of the member's first fork
     evidence or `DeviceRetired` (*amended 2026-11-23, plan E3.2 K4b Q7:* in the **strict** causal past of each op of
     every fork evidence and of every `DeviceRetired` carrier of the member held at the readmit's deps: the
     intersection, not the lowest stamp, so a key endorsed after one op of a fork pair and concurrent with the other is
     not pre-fork, and the exempt replacement carried in a `DeviceRetired`'s op is never pre-fork). A key endorsed after it, a re-admitted key or the exempt replacement below
     included, can never co-sign an own-key `DeviceReadmit`, so approvals do not chain (Signal: only the primary
     approves a link, `DeviceController.java` L204-206; Apple: a newly added trusted device waits before it can
     make critical changes).
   - **One per checkpoint interval.** At most one own-key `DeviceReadmit` per member counts per checkpoint
     interval, the span between two consecutive checkpoint heights of plan E3.2 S7c (any cut a strict majority
     endorses per ADR 0018; checkpoints are acceleration hints and decide no op's fate, *Corrected 2026-10-24 (pass 3)*, from "a
     deterministic function of a stable causal cut"). Of a member's own-key readmits whose deps hold the same
     newest checkpoint height with a strict majority, only the lowest `(lamport, op_hash)` counts and the others are inert; the device asks
     again after the next checkpoint. This adapts Signal's per-account link bucket (`RateLimiters.java` L28-29, 6
     refilled one per 2 min) to causal order, a Rekindle choice from that source. The moderator path has no such
     limit, since a person outside the member decides it.
   - **Shown.** Every own-key `DeviceReadmit` raises a notice on all of the member's devices and to members
     holding `KICK_MEMBERS`, naming the co-signing key and the fork evidence (Matrix MSC4153: the user's own
     devices MUST alert; Apple sends the request to every trusted device).
   - **A fork event's evidence** (*amended 2026-11-23, plan E3.2 K4b.4, R1 and R2:* Keybase's sigchain names the
     reset it follows, Matrix MSC1756/MSC4153 cross-signing names the key it vouches for). A member's fork events are
     each op of a fork pair (rule 5) and each `DeviceRetired` carrier (rule 4); the retirement is left out only of
     the second-fork test below, so the equivocate, retire, repeat loop still buys one free round. A fork event's
     evidence is the fork pair's hash (rule 5) for an op of a pair, and the retirement evidence (rule 10) for a
     `DeviceRetired` carrier. A key endorsed after the member's first fork event in an op's deps is effective
     there only with an applied `DeviceReadmit` naming it, its `forkEvidence` the evidence of a fork event of the
     member; the writer names its lowest-stamp fork event's. Every `DeviceReadmit`, own-key and moderator alike,
     whose `forkEvidence` is no fork event of its member held at its deps is refused, so a co-signature always
     answers a fork the reader can see. The exempt replacement as its own additive op (`DeviceReEnrol`, the new
     key's seq 0, exempt iff its strict past holds no fork event of the member) is R3, pending plan E3.2
     S7d-keys.10; until then the replacement needs a co-signer like any other post-fork key.

   A **second** fork from any of that member's keys, pre-endorsed ones included, retires **all** its keys (*amended
   2026-11-23, plan E3.2 K4b Q9:* on the converged set, every key `k` of the member is void above its floor, the
   highest seq of `k` in the causal past of any op by another member's key, a quarantined one included, as rule 5's
   floor keeps what others built on; forked keys are also void from their fork seq), after
   which only the moderator path (rule 8) remains, or, for a creator, a `CommunitySuccessor`; so pre-endorsing keys
   buys at most one fork. Exempt from the co-signer is only the replacement key that the member's first
   `DeviceRetired` is carried with (rule 4's restore or clone signal, one op carrying both; *amended 2026-11-23, plan
   E3.2 K4b.4 R3:* carried as one `DeviceReEnrol{retired, endorsement}` op at the new key's seq 0, exempt iff its
   strict past holds no fork event of the member, pending plan E3.2 S7d-keys.10). When an op's deps hold
   two carriers of a `DeviceRetired` for that first retired key (two copies retiring it concurrently), the exempt
   replacement is the one in the lowest `(lamport, op_hash)` carrier, and the other copy's replacement needs a
   `DeviceReadmit` for every op whose deps hold both (ops already judged stay judged, rule 6). A later
   `DeviceRetired` retires only its own key, but its replacement needs a co-signer: a `DeviceRetired` opens the first
   fork but is never a second fork, which is fork evidence for a second distinct key (*amended 2026-11-23, plan E3.2
   K4b, order-free:* fork evidence on `d2` is a second fork iff fork evidence on another key of the member exists, or
   a fork event of another key is in the strict past of both of `d2`'s fork ops; on the converged set the first
   clause decides, so it never depends on arrival order) (corrected 2026-10-24, premise
   check), so retiring a lost device after a restore retires no other key. A self-signed reset is
   evidence shown to the co-signer, never a key on its own (Keybase re-add, Autobase `addWriter`). Retiring a key
   does **not** retire the keys it co-signed (Keybase revokes only the named key, `keyfamily.go` L752-810, and
   effectiveness judged at causal deps gives the same): a co-signee of a key later found compromised needs its own
   `DeviceRetired` or a moderator's review, and the retire and review screens list the keys it co-signed. Rule 7
   is not theft protection: a thief holding the master secret satisfies the own-key path, and theft recovery is
   rule 0's identity reset.
8. **Re-admission follows the kick rule** when the member has no effective device key left (every key retired),
   and whenever another member co-signs. That `DeviceReadmit` must be authored by another member's effective
   device key whose member holds `KICK_MEMBERS` and **outranks the target's highest role** in the MSC4289 hierarchy
   built at S3 (exactly the rank check a kick needs). The re-admission restores membership with only the target's
   roles at or below the co-signer's rank; roles above it are not restored. Nobody outranks a creator (0011 :77-81),
   so a **creator** whose device keys are all retired recovers through a creator-signed `CommunitySuccessor` (0011
   :83-96): from a co-creator's or its own remaining effective device key, or, when the creator has none, signed by
   the creator pseudonym (rule 10). A successor is a positioned, one-shot effect, and `p` exists on every device
   and after every restore, so the pseudonym-signed form has its own rules:
   - it is harvested from any op that carries it, like an endorsement, and takes effect in **one canonical op**,
     the lowest `(lamport, op_hash)` op carrying it;
   - it is **write-once per (community, causal cut)**: it names the cut at which the creator has no effective
     device key (the frontier holding the fork evidence or `DeviceRetired` it answers), and its condition is
     judged at that cut;
   - it carries `forkEvidence`: the fork-evidence hash, or, when the keys were retired by a restore and not a
     fork, the "retired by restore" marker, `BLAKE3 derive_key("rekindle-gov-restore-evidence-v1")` over
     `community ‖ deviceKey ‖ incarnation ‖ lastSeq ‖` the hash of the published `GovHead` op the barrier read;
   - two conflicting pseudonym-signed successors from one creator (different content at the same cut, or at
     concurrent cuts) **void each other**: both are shown, neither is offered, and the successor then needs the
     co-signed path, a co-creator's effective device key.
9. **Both sides see it; nothing resolves itself.** The forked member sees that its device wrote from an old backup
   and must ask a member to re-admit it. Other members see the conflicting pair, a "devices changed" notice when a
   member's effective device set changes, and a flag on a member with a forked device. There is no automatic
   recovery path (threat model, "no creative paths").
10. **Signed layouts.** Every signature binds the community and the member record, so nothing replays across
    communities, members or forks:
    - endorsement, by `p`: `rekindle-gov-device-key-v1 ‖ community ‖ memberRecord ‖ deviceKey ‖ incarnation ‖
      extendsSet ‖ deviceRecord ‖ deviceRecordOwner ‖ forkEvidence`, where `memberRecord` is `M(p)` from
      `MemberJoined` and `forkEvidence` is the hash of the fork evidence the endorsement answers, or 32 zero bytes
      when it answers none;
    - `extendsSet` (bound in the endorsement): `BLAKE3 derive_key("rekindle-gov-device-set-v1")` over the sorted
      ids `(kind, deviceKey, incarnation)` of every element of the member's set the signer held;
    - retirement, by `p`: `rekindle-gov-device-retired-v1 ‖ community ‖ memberRecord ‖ deviceKey ‖ incarnation ‖
      lastSeq`;
    - retirement evidence (*amended 2026-11-23, rule 7*): `BLAKE3 derive_key("rekindle-gov-retirement-evidence-v1")`
      over `community ‖ memberRecord ‖ deviceKey ‖ incarnation ‖ lastSeq` of a `DeviceRetired` (text fields
      length-prefixed, integers big-endian, as the fork-evidence hash), the `forkEvidence` a post-fork endorsement
      and its `DeviceReadmit` name when the fork event they answer is a retirement;
    - pseudonym-signed successor, by the creator's `p`: `rekindle-gov-successor-v1 ‖ community ‖ cut ‖ newAnchor ‖
      forkEvidence`, where `cut` is the hash of the frontier it names (rule 8);
    - re-admission: the `DeviceReadmit` op under `rekindle-gov-op-v1` (which already binds the community), whose
      body is `rekindle-gov-device-readmit-v1 ‖ community ‖ memberRecord ‖ newKey ‖ incarnation ‖ forkEvidence`, so
      a co-signature answers one fork and one incarnation and cannot be reused for a later fork;
    - an endorsement whose `forkEvidence` is non-zero is effective only with a `DeviceReadmit` naming the same
      `newKey`, `incarnation` and `forkEvidence`.

## Consequences

- ADR 0011's write model holds: member records owned by random keys only the member's devices hold, never by the
  pseudonym (R5). What changes is the op signature: ops name a device key, and the reader's check becomes "signed
  by a device key `p` endorsed for this community and record, effective and not retired" instead of "signed by
  `p`".
- ADR 0011's `MemberRecordMoved` (:150) is replaced: a lost or compromised device is `DeviceRetired` plus a new
  `DeviceKey` with a new `M(p,d)`; the record moves with the device key. A compromised **master secret** is not
  helped: its holder can retire every honest device and endorse its own, as it can impersonate today (A6, A8,
  Z9); the "devices changed" notice is the only signal, and the remedy is an identity reset (rule 0).
- D15's quarantine is bounded to the device key and to ops at or above the fork seq; D15's equivocation bullet is
  amended to say so. D15 equivocation applies to device keys only; `p`'s endorsement set cannot equivocate.
- New ops `DeviceKey`, `DeviceRetired` and `DeviceReadmit` with fresh ordinals and power rows; new domain labels
  `rekindle-gov-device-key-v1`, `rekindle-gov-device-retired-v1`, `rekindle-gov-device-readmit-v1`,
  `rekindle-gov-device-set-v1`, `rekindle-gov-fork-evidence-v1`, `rekindle-gov-restore-evidence-v1`,
  `rekindle-gov-retirement-evidence-v1` (*amended 2026-11-23, rule 7*; the `deviceReEnrol @52` arm follows with R3,
  plan E3.2 S7d-keys.10) and `rekindle-gov-successor-v1`; new intent rows for re-admission (ADR 0019);
  a vault entry `DeviceOpKey{community}` that is never exported; the governance stores keyed by device key.
- The genesis signs under the creator's first device key: the `GenesisCert` carries the creator's first
  endorsement, so origin needs no separate op.
- ADR 0018 ballot votes are ops, so a restored device votes under its new key; the electorate is counted per
  member, and a member whose device keys are all retired is not in it until re-admitted (ADR 0018, 2026-10-24).
- `DeviceRetired`'s power row is "self only", meaning `p`'s signature: it is valid from whatever op carries it,
  a quarantined op or one signed by a forked, retired or unendorsed key included, so the authority check (`rekindle-governance/src/validate/mod.rs:113` `permitted`, built at plan S3;
  corrected 2026-10-24, pass 2, from "the power table (`power.rs`)", whose `class_of` only sorts an op into power
  and non-power and refuses nothing)
  judges it by `p`'s signature and never refuses it on its carrier's standing. Only `p` can retire, so this is
  harmless.
- Restores count against the one-fork rule (rule 7): an honest user's first restore is free, but after it every
  new device, and any second restore, needs a co-signer: one of the member's own pre-fork device keys (as a
  Signal device approves a link), at most once per checkpoint interval and shown to the member's devices and to
  `KICK_MEMBERS` holders, or a `KICK_MEMBERS` holder who outranks the member. A second fork retires all
  the member's keys, and then only the rank path (rule 8) or, for a creator, `CommunitySuccessor` remains. That
  is the cost of bounding equivocation to one fork per member; the only rate window is the own-key readmit's,
  adapted from Signal's link bucket (*amended 2026-10-24, prod research batch 10*). A member whose data root was
  copied and who has no other pre-fork key keeps one copy on the exempt replacement; the other copy re-enrols
  through the rank path, and a sole creator's second copy stays unable to sign in that community.
- A copied data root is caught by either copy, seeing an op under its own key that it did not write, or by the
  check before signing (rule 4). The key is retired for both copies and each re-enrols with every copied
  per-device secret replaced; ops above `lastSeq` are quarantined and shown, and their author re-sends them
  under the new key.
- A device signs seq n + 1 only after seq n's append landed (rule 4), so its governance writes are serialized
  behind DHT landing: two quick admin actions from one device are at least one DHT set apart, on top of S6b's
  landed-path command deadline. This latency is the price of never having a gossip-only head.
- Built as E3.2 slice S7d-keys, after S7c; E3.3's `M`/4 entries and E3.4's beats are signed by the device key of
  the record they sit in.

## More information

- [0011](0011-member-sovereign-records.md), [0018](0018-mek-rotation-ballots.md)
- Keybase sigchain resets: `go/protocol/keybase1/reset.avdl`, `go/teams/chain.go`
- [RFC 9162](https://www.rfc-editor.org/rfc/rfc9162) §8 and §11.3, Certificate Transparency v2
- Matrix cross-signing and device keys (Client-Server API, end-to-end encryption module)
- Hypercore `core.js` and Autobase `system.js`; Secure Scuttlebutt `ssb-validate`
- [MSC4289](https://github.com/matrix-org/matrix-spec-proposals/pull/4289), the creator above every role
- [MSC4153](https://github.com/matrix-org/matrix-spec-proposals/pull/4153), exclude non-cross-signed devices: a
  device that is not cross-signed is not trusted (rule 0)
- [MSC4312](https://github.com/matrix-org/matrix-spec-proposals/blob/de9948c35da30f4c172d1c727fdf3bb4946e1f0c/proposals/4312-x-signing-reset-with-nextgen-auth.md#L1-L17),
  the cross-signing reset, behind extra authentication and seen by peers as an identity change (rule 0); prod
  research batch 3
- Signal-Server `AccountsManager.java` L204 at commit `8edf3488f7d97398db7ffa5184c6b769a272ff59`, `MAX_DEVICES`
  (rule 6); prod research batch 7, 2026-10-24
- Prod research batch 10, 2026-10-24 (copied data root, rules 4 and 5; re-admission, rule 7):
  - Signal-Server at `8edf3488f7d97398db7ffa5184c6b769a272ff59`: `controllers/DeviceController.java` L204-206
    (only the primary creates a link token), `limits/RateLimiters.java` L28-29 (`AllocateDeviceLimiter` and
    `VerifyDeviceLimiter`, a bucket of 6 refilled one per 2 min), `websocket/WebSocketConnection.java` L147-154
    (close 4409 "Connected elsewhere"), `websocket/WebSocketDisconnectionRequestListener.java` L28-32 (close 4401
    "Reauthentication required"); Signal-Desktop at `17502b9d1a9e34af689f31508c4ac4ccee5b5cb7`
    `ts/textsecure/SocketManager.preload.ts` L389-393 (no reconnect after 4409); libsignal at
    `fc90b90fa2b9c37822fa47ba0e0dcdf387a0c071` `rust/net/src/chat/ws.rs` L140-146 (`ConnectedElsewhere`)
  - Tailscale at `a6cffd0a6d11ef8aa3a0ca0ddc15879d0845605d`: `derp/derpserver/derpserver.go` L110-117, L262-273,
    L1883-1910 (duplicate clients, `lastWriterIsActive` and `disableFighters`), `ipn/ipnlocal/tailnet-lock.go`
    L253-260, L314-334 (`rotationTracker`, both clones rejected on a tie), `ipn/ipnlocal/local.go` L4604-4611,
    L8628-8637 (`--force-reauth` keeps the machine key), `cmd/tailscaled/tailscaled.go` L220, L237-240
    (`--encrypt-state`, `--hardware-attestation`); wireguard-go at `2631ce99a06f27120d581611cf125d68bc6aa565`
    `device/receive.go` L374 and `device/noise-protocol.go` L686-704 (endpoint and keypair flapping)
  - matrix-rust-sdk at `8de215a0aa0026afa5754ae5f05a6d168d1a1d60` `crates/matrix-sdk/src/encryption/mod.rs`
    L531-540, L825-880 (duplicate one-time key) and L2010-2091 (cross-process store lock); Element Web at
    `15f63d5959697796e9d30d507e75513b6c97e34f` `apps/web/src/utils/SessionLock.ts` L10-29
  - Keybase client at `d87bde33c55dfa49ba47d95f15a745b87a97526d`: `go/libkb/keyfamily.go` L752-810, L917-939
    (revoking a device revokes only its own keys), `go/engine/kex2_provisioner.go` L336-380 (any sibkey provisions)
  - Apple Stolen Device Protection, https://support.apple.com/en-us/120340 (adding or removing a trusted device
    away from familiar locations needs Face ID or Touch ID and an hour's wait), and
    https://support.apple.com/en-us/122621 (a newly added trusted device waits before critical changes), both
    retrieved 2026-10-24
- Threat model L1, I8, I14, Z2, Z9; rows S6, T7, E6, E7 (added with this ADR)
- Register `audit-2026-10-23/00-register.md`, "Decided 2026-10-24 from the research", item Q7
- Reviews, 2026-10-24, held outside the repo: of the first draft (record owner, endorser, endorsement set,
  per-device records, rank rule, cap and rate limit, signed layouts, barrier); of the rewrite (causal-deps
  effectiveness, set-extending endorsements, one fork per member, the barrier over answering peers, successor
  conflicts, card version); of the companion docs (the corrections in the status line)

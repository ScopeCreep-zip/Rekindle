# 0011 — Member-sovereign records replace the shared-`slot_seed` write model

- **Status:** Accepted
- **Date:** 2026-10-04
- **Supersedes:** the write model of [0003](0003-flat-smpl-governance.md), i.e. universal SMPL
  `o_cnt: 0` records whose slot keypairs every member derives from a community-shared `slot_seed`
  (AUTH §8.3 "shared knowledge by design"). Flat governance, CRDT merge, reader-side permission
  validation and self-sovereign join from 0003 **stand**.

## Context and problem statement

Under 0003, every member derives every slot's writer keypair from one `slot_seed` carried in
invites (`crates/rekindle-secrets/src/derive.rs:86-92`: "any member can derive any slot's keypair
— this is by design"). The architecture accepted this because "authenticity comes from the
pseudonym signature".

Signatures prevent forgery. They do not prevent **erasure or permanent locking**:

- veilid-core 0.5.7 storage nodes accept any signed value with a higher sequence number
  (`storage_manager/set_value.rs:702-720`).
- `ValueSeqNum::MAX = u32::MAX − 1`, and `next()` fails at MAX (`value_seq_num.rs:16,25-30`).
- Veilid's developer book warns about exactly this pattern: a well-known writer key lets "anyone
  … [write] a subkey at the maximum sequence number and [close] it to future writes".

So any one member can unattributably and permanently freeze any other member's governance subkey,
channel page or presence row. That includes governance subkey 0, the genesis, after which no one
can ever join again. For communities of vulnerable users this is an unacceptable insider denial of
service.

There are two further constraints:
- The SMPL member set is fixed at record creation ("Once created, the schema cannot be changed,
  because it is hashed to form part of the record key"), so per-member keys cannot be added to an
  existing SMPL record.
- Record keys are not re-derivable: every create adds a random encryption key
  (`create_record.rs:86-95`).

## Decision drivers

- No member can erase, lock or rewrite another member's data.
- Keep flat governance, with no coordinator and no privileged node.
- Keep self-sovereign join: joining works from DHT reads alone, with nobody online.
- Bounded watch and bootstrap costs (32 anonymous watchers per record; app_call ≤ 32768 B).

## Considered options

- **R1 — Keep `slot_seed` and verify payload signatures.** Stops forgery, not erasure or locking.
- **R2 — VeilidChat-style per-epoch SMPL rosters with per-member keys.** A join must wait for an
  existing member to mint the next epoch, which is not self-sovereign. Each epoch costs O(N)
  rewrites, and sealing needs `minSeqnum`, which 0.5.7 lacks.
- **R3 — Joiner-minted SMPL segments.** O(N) migration per join, and concurrent joins fork.
- **R4 — Slots pre-assigned at invite time.** Turns the inviter into a privileged, must-be-online
  minter.
- **R5 — Member record owned by the pseudonym key.** Re-creation collides; it reuses signing keys
  across protocols and links storage writers to pseudonyms.
- **R6 — One record per (member, channel).** C×N records and watches.
- **R7 — Every member watches every record.** Exceeds the 32-watcher limit.
- **R8, R9 — Shared notification or doorbell records.** A shared writer key is again a lock target.
- **R10 — Relay-signed membership.** A privileged node, which the architecture forbids.
- **MSR — Member-sovereign records (selected).**

Detail: `.claude/plans/standards-remediation/evidence/slot-integrity.md` §5.

## Decision outcome

**Chose member-sovereign records.**

**Community anchor `A`.** A DFLT(1) record. Its random owner key `K_A` writes a `GenesisCert`,
signed by `K_A` and by the creator's pseudonym, and is then zeroized and never persisted.
- Readers pin the record descriptor's owner to the certificate signer.
- The anchor is physically immutable.
- The community id is `A`'s record key.

**MemberRecord `M(p)`.** One DFLT(8) record per member pseudonym per community, owned by a random
key only that member holds (vault `MemberRecordOwner{community}`).

| Subkey | Content |
|---|---|
| 0 | member card |
| 1 | presence beat |
| 2 | governance op-log head |
| 3 | DAG frontier |
| 4 | channel outbox (blinded channel tags + spill table) |
| 5 | history advertisement |
| 6–7 | reserved |

Veilid accepts writes only from the owner, so nobody else can erase or lock `M(p)`.

**Sealed pages.** Spill pages, op-log pages and directory pages are DFLT(1) records created with a
fresh random owner, written once, and the key is discarded.

**Binding.** A self-authored `MemberJoined` op carries `memberRecord` and `memberRecordOwner`.
Readers accept data from `M(p)` only if the descriptor owner matches and the inner pseudonym
signature verifies.

**Discovery.**
- Op dependencies carry locators (`OpRef`), so any op is fetchable without a directory.
- Checkpoint ops list directory pages of about 170 members each.
- Bootstrap over app_call is an accelerator of about 8.4 KiB, constant in N. A joiner can always
  rebuild the member set from the DHT.

**Notification.**
- Gossip is the fast path.
- Each member watches 4 ring successors, giving ≤ 4 watchers per record at any size, and relays
  content-free `HeadAdvanced` hints.
- A 60-second anti-entropy inspect sweep.

**Join.**
1. Create `M`.
2. Read the anchor and the inviter's frontier.
3. Walk the DAG.
4. Publish card, `MemberJoined`, frontier and beat to one's own `M`.

There is no slot claim and no approval loop.

**Leave, kick, ban.** These are ops plus a departed beat in one's own record. Moderation never
writes another member's data. A kicked member's own writes are dropped by readers
(reader-validates).

**Key loss or compromise.** The member appends a `MemberRecordMoved` op.

**Multiple devices.** Each device gets its own `M(p,d)`, bound by an op, so devices never race on
one subkey.

## Consequences

**Positive.**

- No insider can erase or freeze another member's data or the genesis.
- No 255-member slot ceiling and no segment expansion; scale comes from directory pages.
- Watch cost is constant per member, and today's over-limit `Rejected` watches disappear.
- Joining works with nobody online.

**Negative.**

- About N records per community instead of roughly 3 + C. Members keep peers' records alive by
  opening them (each Veilid node stores at most 128 remote records). VeilidChat ships the same
  per-participant pattern.
- Reading a channel costs O(authors who posted in it).

**Deleted.**
- `derive_slot_keypair`, `slot_seed`, `SlotSeed`.
- The registry record and per-channel SMPL records.
- Segments: `SegmentAdded`, `ChannelSegmentLinked`, `RequestSegmentExpansion`.
- Overflow chains, `DHTLog`/`DHTShortArray`.
- Owner-key sharing (`@39 adminKeypairGrant`).

## More information

- `.claude/plans/standards-remediation/evidence/slot-integrity.md` — design, watch and size
  budgets, rejected alternatives
- `.claude/plans/standards-remediation/00-integration-plan.md` D15, steps E3.2–E3.4
- [Veilid developer book](https://veilid.gitlab.io/developer-book) — DHT writes and sequence numbers
- [0003](0003-flat-smpl-governance.md)

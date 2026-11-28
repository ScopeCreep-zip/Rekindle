# 0011 — Member-sovereign records replace the shared-`slot_seed` write model

- **Status:** Accepted; **implemented** for governance (plan E3.2, 2026-11-18): the anchor and
  `GenesisCert`, DFLT(8) member records, the signed op DAG and its merge, checkpoints with the
  pre-join hint and partial state, the k = 4 watch ring and anti-entropy sweep, and the
  self-sovereign join and re-join (`crates/rekindle-governance/`, `crates/rekindle-governance-runtime/`,
  `docs/architecture/communities-governance.md`). Channel records and the registry slot claim keep
  the `slot_seed` model until plan E3.4.
- **Date:** 2026-10-04
- **Supersedes:** the write model of [0003](0003-flat-smpl-governance.md), i.e. universal SMPL
  `o_cnt: 0` records whose slot keypairs every member derives from a community-shared `slot_seed`
  (AUTH §8.3 "shared knowledge by design"). Flat governance, CRDT merge, reader-side permission
  validation and self-sovereign join from 0003 **stand**.
- **Amended 2026-10-22** (plan §0, Q3 and Q6 decided from research): the `GenesisCert` names a fixed
  creator set (`additionalCreators`) and a `predecessor`, and a creator-signed `CommunitySuccessor`
  op is the recovery and ownership-transfer path (Q3); an `ApprovalRequired` community admits a
  joiner by a reader-validated `MemberApproved`, and channel keys are withheld until then (Q6).
- **Amended 2026-11-14** (plan step VU, Veilid 0.5.8; citations below re-read in the pinned fork at
  `91453e97`, ADR 0017): 0.5.8 adds `SetDHTValueOptions.min_seqnum` ("raise the sequence number to at
  least this", `veilid_api/types/dht/set_dht_value_options.rs:12-18`; applied at
  `storage_manager/set_value.rs:384-385`), so the "0.5.7 lacks `minSeqnum`" ground under R2 is gone. **R2
  stays rejected on its other grounds:** a join waits for an existing member to mint the next epoch (not
  self-sovereign) and each epoch costs O(N) rewrites. The insider-lock threat in the context below gets
  **easier**, which strengthens this decision: any holder of a well-known writer key now locks a subkey with
  one write at `min_seqnum = ValueSeqNum::MAX`, no walk up the sequence numbers needed, and a storage node
  refuses every successor while it holds that value (`set_value.rs:797-804`). 0.5.8 also makes the
  local seq bump `saturating_next` (`value_seq_num.rs:35-39`), which saturates at MAX instead of failing.
  The 2026-10-04 text is kept; the amended passages say what changed.
- **Amended 2026-10-24** (plan E3.2 S7c and S7d-keys premise check; ADR 0022, Proposed): directory pages hold 128
  entries (Discovery), conflicting pseudonym-signed successors are not offered (Successor community), record data is
  checked against its device key (Binding), and each device also watches its own record (Notification).
- **Amended 2026-11-23** (plan RV2d, D16): the MemberRecord's reserved subkeys are taken. Subkey 6 holds the
  member's routes (plan E3.2 S7c.8) and subkey 7 is its **key drop**: the durable copy of each first community key
  the member owes a joiner, the same D16 seal (an ECDH wrap to the joiner's pseudonym, no recipient field) the
  `app_call` carries. Only `K_M(p)` writes it; a joiner watches its inviter's subkey 7 only until it holds a key,
  tries each entry (another joiner's fails the unwrap quietly) and installs through the call path's own receive;
  a delivered key leaves the drop.

## Context and problem statement

Under 0003, every member derives every slot's writer keypair from one `slot_seed` carried in
invites (`crates/rekindle-secrets/src/derive.rs:86-92`: "any member can derive any slot's keypair
— this is by design"). The architecture accepted this because "authenticity comes from the
pseudonym signature".

Signatures prevent forgery. They do not prevent **erasure or permanent locking**:

- veilid-core storage nodes accept any signed value with a higher sequence number (0.5.8:
  `storage_manager/set_value.rs:797-809`, `inbound_set_value`; 0.5.7: `:702-720`).
- `ValueSeqNum::MAX = u32::MAX − 1`, and `next()` fails at MAX (0.5.8:
  `veilid_api/types/dht/value_seq_num.rs:15,23-33`; 0.5.7: `:16,25-30`). In 0.5.8 one write with
  `min_seqnum = MAX` reaches it directly (amendment above).
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
  (`create_record.rs:86-95`; 0.5.8 `:89-93`).

## Decision drivers

- No member can erase, lock or rewrite another member's data.
- Keep flat governance, with no coordinator and no privileged node.
- Keep self-sovereign join: joining works from DHT reads alone, with nobody online.
- Bounded watch and bootstrap costs (32 anonymous watchers per record; app_call ≤ 32768 B).

## Considered options

- **R1 — Keep `slot_seed` and verify payload signatures.** Stops forgery, not erasure or locking.
- **R2 — VeilidChat-style per-epoch SMPL rosters with per-member keys.** A join must wait for an
  existing member to mint the next epoch, which is not self-sovereign. Each epoch costs O(N)
  rewrites, and sealing needs `minSeqnum`, which 0.5.7 lacked (0.5.8 adds it; R2 stays rejected on the
  first two grounds, see the 2026-11-14 amendment).
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
- *Amended 2026-10-22 (Q3).* The certificate names the **creator set**: `creatorPseudonym` plus
  `additionalCreators`, at most 8 further pseudonyms, fixed at genesis and immutable like the rest
  of the anchor (no op adds or removes a creator). It also carries `predecessor`, the anchor of the
  community this one succeeds, or none. `state.creator` becomes this set; every creator-only act
  (`validate/mod.rs:112-117`) accepts any member of it.

**Successor community** *(added 2026-10-22, Q3).* An immutable anchor cannot be repaired, so
recovery and ownership transfer go through a new community:

- A `CommunitySuccessor{newAnchor, reason}` op counts only when signed by a creator of the old
  community. A `MANAGE_COMMUNITY` holder cannot sign it: Matrix sets room upgrade above admin "to
  guard against unauthorised privilege escalation by admins" (MSC4289:145-151), and Project Hydra
  gives the same two exits, "establish a new creator by either upgrading the room or creating a new
  one".
- The new `GenesisCert` names the old anchor as `predecessor`; readers check both links (the op in
  the old community and the `predecessor` in the new one) before they treat it as a successor.
- Clients show who signed it and **offer** the move. Nobody is moved silently, no history is
  copied, the old community stays readable, and if two valid successors exist both are shown. *(Amended
  2026-10-24, ADR 0022 rule 8: two conflicting successors signed by one creator's pseudonym void each other; both
  are shown and neither is offered.)*
- Ownership transfer is a successor with a new creator set.
- If every creator key is lost, members create a new community and re-invite. There is no
  member-signed successor: it would let any member phish the whole community.

**MemberRecord `M(p)`.** One DFLT(8) record per member pseudonym per community, owned by a random
key only that member holds (vault `MemberRecordOwner{community}`).

| Subkey | Content |
| --- | --- |
| 0 | member card |
| 1 | presence beat |
| 2 | governance op-log head |
| 3 | DAG frontier |
| 4 | channel outbox (blinded channel tags + spill table) |
| 5 | history advertisement |
| 6 | member routes (the general private route blob every `app_call` to the member uses; plan E3.2 S7c.8) |
| 7 | key drop: the sealed `MekTransfer` envelopes the member still owes joiners (plan RV2d, D16) |

Veilid accepts writes only from the owner, so nobody else can erase or lock `M(p)`.

**Sealed pages.** Spill pages, op-log pages and directory pages are DFLT(1) records created with a
fresh random owner, written once, and the key is discarded.

**Binding.** A self-authored `MemberJoined` op carries `memberRecord` and `memberRecordOwner`.
Readers accept data from `M(p)` only if the descriptor owner matches and the inner pseudonym
signature verifies. *(Amended 2026-10-24, ADR 0022: the inner signature is the device key's that the record's
`MemberJoined` or `DeviceKey` endorsement binds, itself endorsed by the pseudonym.)*

**Discovery.**

- Op dependencies carry locators (`OpRef`), so any op is fetchable without a directory.
- Checkpoint ops list directory pages of about 170 members each. *(Amended 2026-10-24, plan S7c: 128 members
  each, so a page is a complete subtree of one RFC 9162 tree; each entry names the member's record and owner.)*
- Bootstrap over app_call is an accelerator of about 8.4 KiB, constant in N. A joiner can always
  rebuild the member set from the DHT.

**Notification.**

- Gossip is the fast path.
- Each member watches 4 ring successors, giving ≤ 4 watchers per record at any size *(amended 2026-10-24, ADR 0022
  rule 4: each device also watches its own `M(p,d)`/2 to see a copy's write, so ≤ 5)*, and relays
  content-free `HeadAdvanced` hints.
- A 60-second anti-entropy inspect sweep.

**Join.**

1. Create `M`.
2. Read the anchor and the inviter's frontier.
3. Walk the DAG.
4. Publish card, `MemberJoined`, frontier and beat to one's own `M`.

There is no slot claim and no coordinator approval loop; an `ApprovalRequired` community admits by
a reader-validated `MemberApproved`, and channel keys are withheld until then. *(Amended
2026-10-22, Q6; the original read "no slot claim and no approval loop".)* The joiner still writes
its own `M` and `MemberJoined`, which acts as a knock (Matrix MSC2403). Readers count it a member
only after a converged `MemberApproved` by a `KICK_MEMBERS` holder; no channel MEK reaches a
pending member, the converged approval supplies the current generation's wraps (a key supplement,
not a generation bump), and `MemberRejected` drops the pending member's ops.

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

- **Note (plan decision 67, item 3, 2026-11-26): the group rendezvous record.** A group DM's
  durable first-contact path between members who are not friends is a creator-minted
  `SMPL(o_cnt 1, members = 8 one-time writers, m_cnt 8)` record, 65 subkeys, subkey `1 + 8i + j`
  = member i's first contact to member j (`dht_layout::group_rendezvous`). It is the one
  record here that is not member-sovereign: its creator mints every writer. Bound: only W_i
  writes row i (veilid-core 0.5.8 `derivation.rs:191-235`), the sender of an entry is proven by
  its signed `PreKeyInit` and the roster, and the creator can only junk-write (deny the durable
  copy), never forge; the `app_call` copy remains. See threat model D9.

- `.claude/plans/standards-remediation/evidence/slot-integrity.md` — design, watch and size
  budgets, rejected alternatives
- `.claude/plans/standards-remediation/00-integration-plan.md` D15, steps E3.2–E3.4
- [Veilid developer book](https://veilid.gitlab.io/developer-book) — DHT writes and sequence numbers
- [0003](0003-flat-smpl-governance.md)
- Plan §0 decision log, 2026-10-22 rows "Q3 decided" and "Q6 decided"
- [MSC4289](https://github.com/matrix-org/matrix-spec-proposals/pull/4289) — explicitly privileged
  room creators; room upgrade above admin
- [MSC2403](https://github.com/matrix-org/matrix-spec-proposals/pull/2403) — knocking
- [Matrix Project Hydra](https://matrix.org/blog/2025/08/project-hydra-improving-state-res/) —
  creators, room upgrades

# 0012 — Community channel keys stay on MEK + deterministic rotator (MLS re-evaluated)

- **Status:** Accepted
- **Date:** 2026-10-04
- **Re-affirms:** the community-key choice recorded in [0002](0002-signal-protocol-for-1to1.md)
  ("community channels use MEK + reader-validates governance … Revisit when a CRDT-friendly MLS
  variant matures"). Nothing in 0002 is superseded.

## Context and problem statement

0002 rejected MLS (RFC 9420) for community channels because its ratchet-tree state machine
"assumes a coordination point". A later plan draft (2026-10-04, v1 of the standards-remediation
plan) proposed replacing channel MEKs with MLS through the `openmls` crate, using a fork-choice and
finality layer over the governance DAG. 0002's own revisit condition requires checking whether a
CRDT-friendly MLS variant has matured.

## Evidence (primary sources, `.claude/plans/standards-remediation/evidence/`)

- **DCGKA** (Weidner, Kleppmann, Hugenroth, Beresford, ACM CCS 2021; `raw/dcgka.txt`): existing
  secure-group-messaging work "targets a centralized network model in which all messages are routed
  through a single server, which is trusted to provide a consistent total order on updates to the
  group state". DCGKA is the published construction for decentralised groups.
- **Decentralised MLS** (`draft-kohbrok-mls-dmls`, `raw/draft-kohbrok-mls-dmls.txt`) is an
  individual draft that expired on 2026-04-23. It says "applications should take care that group
  state forks are short-lived" and that long-lived forks should be avoided. A serverless network
  with offline members produces exactly such forks.
- **Matrix MSC2883** (MLS for Matrix) is described upstream as still at "braindump stage".
- **`p2panda-encryption`** (the DCGKA-derived Rust implementation, MIT/Apache, `raw/p2panda-enc-README.md`)
  says of itself: "has not yet received a security audit"; "We currently *cannot* recommend using
  this technology for high-risk use-cases"; group control messages are unencrypted; there is no
  post-quantum protection.
- **Verification of the MLS plan** (`steps-27-34.md` step 30) found it unsound as written:
  - `max_past_epochs` does not retain exporter secrets;
  - its finality rule is open to Sybil attacks and stalls when members are idle;
  - publishing Welcome messages before finality violates RFC 9420 §14 and RFC 9750 §5.2.3.

## Decision outcome

**Keep 0002's community-key model:**
- per-channel MEKs (AES-256-GCM) and a community-scope key;
- rotation by the deterministic rotator;
- X25519 wraps delivered peer to peer;
- private channels enforced by withholding wraps from members without read access.

**Harden it** (plan D11, D16, D20; steps B5, E3.5):

- **Signed wraps.** MEK wraps are signed by the rotator's pseudonym and checked against governance
  rank, which closes the unsigned-MEK injection.
- **Rotator election.** Only members holding KICK, BAN or MANAGE_COMMUNITY are eligible; if none
  is online the rotation waits.
- **Generation rule.** A `MEKGenerationBump` is valid only as `generation == current + 1`.
- **Durable kicks.** A kick is a durable `MemberRemoved` governance op that triggers rotation.
- **Shared codec.** One channel-body codec with AAD for every host, in `rekindle-secrets::channel_body`.
- **History for joiners**, following the Matrix MSC4268 pattern (spec v1.19):
  - each key generation is stamped at use time with the channel's history visibility;
  - joiners receive the generations their role may read;
  - a visibility change forces rotation.
- **Private channels.** A deterministic `channel_read_roster` decides which members receive wraps,
  and bootstrap stops shipping every MEK.

**Revisit trigger (unchanged from 0002, made concrete).** A decentralised group key agreement
qualifies when it has:
- a published security analysis;
- an audited implementation under a licence compatible with MIT;
- support for partition-tolerant, offline-heavy groups.

DCGKA-family implementations are tracked against this.

## Consequences

- Removes a large dependency (`openmls` plus `hpke-rs`, `tls_codec`, P-256/384, `rayon`) and a
  consensus layer the network cannot provide.
- Post-compromise security for channels comes from rotation on departure, kick and ban, plus
  scheduled rotation. That is weaker than a continuous group ratchet, and the threat model states
  it.
- Group DMs keep their documented design: 3–8 members, wrapped MEK, rotation on leave.

## More information

- [0002](0002-signal-protocol-for-1to1.md), [0011](0011-member-sovereign-records.md)
- [`../architecture/communities-channels.md`](../architecture/communities-channels.md)
- [DCGKA paper](https://eprint.iacr.org/2020/1281.pdf), [MSC4268](https://github.com/matrix-org/matrix-spec-proposals/pull/4268)

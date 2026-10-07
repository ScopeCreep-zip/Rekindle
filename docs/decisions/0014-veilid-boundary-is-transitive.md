# 0014 — The Veilid boundary is transitive

- **Status:** Accepted
- **Date:** 2026-10-07
- **Supersedes:** the boundary sentences of [0001](0001-veilid-as-transport.md) ("`rekindle-protocol`
  (desktop) and `rekindle-transport` (daemon track) are the only `veilid_core` importers") and of
  [0009](0009-crate-harvest-tiers.md) ("Adding `veilid-core` to any crate other than
  `rekindle-protocol` or `rekindle-transport` requires an ADR"). The rest of 0001 and 0009 stands.

## Context and problem statement

0001 and 0009 confine Veilid to two crates, and `cargo xtask check-boundaries` enforces that by
checking **direct** dependency edges. Both are true of direct imports and false of what actually
links.

The wire types (`CommunityEnvelope`, `SignedEnvelope`, `ChannelMessage`, `MessagePayload`,
`VoicePacket`, `SLOTS_PER_SEGMENT` and the `dht_layout` constants) live in `rekindle-protocol`, and
`rekindle-protocol` always links veilid-core. Any crate that names a wire type therefore links
veilid-core. Measured on 2026-10-07 with `cargo tree -p <crate> -e normal -i veilid-core`, 16
workspace crates link it:

- by design: `rekindle-protocol`, `rekindle-transport`, `rekindle-node`;
- transitively, through the wire types: `rekindle-calls`, `rekindle-channel`, `rekindle-dm`,
  `rekindle-files`, `rekindle-gossip`, `rekindle-governance-runtime`, `rekindle-mek-rotation`,
  `rekindle-presence`, `rekindle-sync`, `rekindle-video`, `rekindle-video-libvpx`,
  `rekindle-voice` and `rekindle-e2e-server`.

So a tier crate meant to be pure (governance runtime, presence, voice) compiles veilid-core, can
reach its API, and pins its version. The architecture audit records this as V27
(`.claude/plans/architecture-audit/01-gap-matrix.md`, "Wire types outside the Veilid crate").

## Decision drivers

- **Upstream churn stays in the adapter layer.** A veilid-core release should rebuild and risk the
  crates that call Veilid, not the domain logic.
- **The boundary must be checkable.** A rule that only direct edges are checked against is not a
  boundary; the check has to see what links.
- **No format change.** Moving a type must not change a byte on the wire or on disk.

## Considered options

1. **Keep the direct-edge rule** and accept transitive linkage. Rejected: the stated boundary is
   then false, and nothing stops a tier crate from calling Veilid through a re-export.
2. **Feature-gate veilid-core inside `rekindle-protocol`.** Rejected: every consumer of the types
   would have to remember to switch it off, and one that does not re-links it silently. The types
   would still sit in the Veilid crate.
3. **Move the wire types out of the Veilid crate, and make the check transitive.** Chosen.

## Decision outcome

**The boundary is defined on transitive linkage, not on imports.**

- The wire types move into `rekindle-codec` (encodings) and `rekindle-types` (layouts and
  constants). `rekindle-protocol` keeps only Veilid calls (the record pool, `RouteImports`,
  `OwnRoutes`, node startup).
- Only these crates may link veilid-core: `rekindle-protocol`, `rekindle-transport`,
  `rekindle-node` (the host), and, until F1 makes it a thin client, `rekindle-desktop`.
- `cargo xtask check-boundaries` becomes transitive: it runs
  `cargo tree -e normal -i veilid-core` per crate, and fails on any crate outside that list. The
  check is mutation-tested (a planted dependency must fail it).
- Within the allowed crates, which code may call which Veilid primitive is rule B22
  (`cargo xtask check-veilid-dht-calls`): DHT calls only in the record pool, imports only in
  `RouteImports`, own-route allocation and release only in `OwnRoutes` and the relay offer.

The move is plan step C8 (`.claude/plans/standards-remediation/00-integration-plan.md`, D29).

## Consequences

**Positive.**

- A tier crate that does not call Veilid cannot link it, and the check proves it on every build.
- A veilid-core upgrade touches three crates (four until F1).
- The wire types become reusable by anything that encodes or decodes Rekindle data without a node,
  such as the e2e server and future tooling.

**Negative.**

- Each tier crate that depended on `rekindle-protocol` only for types re-points its imports (C8
  lists them). This is one pass with no behaviour change.
- The transitive check runs `cargo tree` per crate, which is slower than the direct-edge scan.

## More information

- `.claude/plans/standards-remediation/00-integration-plan.md`: D29, V27, step C8.
- `.claude/plans/architecture-audit/01-gap-matrix.md`: "Wire types outside the Veilid crate".
- [0001](0001-veilid-as-transport.md), [0009](0009-crate-harvest-tiers.md).
- `docs/contributor/architecture-rules.md` rule B22.

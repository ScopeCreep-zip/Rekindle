# Communities — Chiral Network Architecture (v2.0)

Rekindle Communities deliver Discord-class group chat — text
channels, voice, roles, permissions, threads, forums, events,
reactions, moderation, file sharing, rich presence — on the Veilid
peer-to-peer network with **zero servers, zero coordinators, and
zero privileged nodes**. Every participating node is structurally
identical. Governance is not delegated; it is computed.

The Communities spec is split across three documents. Each is sized
to be read in one sitting; together they replace the original
`communities.md` mega-document.

## [`communities-overview.md`](communities-overview.md)

The model and its surroundings:

- The Chiral Network idea, the Schwarzschild Principle, why v2.0
  replaced the v1 coordinator model.
- Three-path delivery (SMPL write, gossip, watch / inspect) and the
  chiral notification model.
- The universal SMPL schema (`o_cnt: 0`, 255 slots), record layout
  per community, subkey overflow with the continuation chain.
- Strand Relay and the five mutual-aid patterns.
- Permission bitfield layout and the resolution algorithm.
- Honest tradeoffs versus Discord, features intentionally omitted,
  the twelve design principles.

Read this first if you are new to the architecture.

## [`communities-channels.md`](communities-channels.md)

The channel-level surfaces:

- MEK lifecycle — peer-to-peer distribution, deterministic rotator
  selection, cascading fallback, per-channel and voice MEK
  rotation.
- Channel messaging via the Tier-7 `rekindle-channel` crate and the
  Tauri-side `channel_adapter` integration.
- Voice, video, and stage channels — mutual-aid SFU election,
  stage hand-raise, video framing through `rekindle-video`,
  Phase 14.q active call registry.
- Direct messages and group DMs — deterministic DM MEK derivation,
  group DM constraints, ratcheting.

Read this when implementing or auditing channel-level features.

## [`communities-governance.md`](communities-governance.md)

The governance internals:

- CRDT merge rules per entry type, the reader-validates principle,
  genesis validation, circular dependency resolution.
- The split between the pure `rekindle-governance` crate and the
  async `rekindle-governance-runtime` crate (origin, bootstrap,
  join, segments, apply).
- Self-sovereign join — invite structure, the 15-step join
  sequence, the BootstrapBundle porter pattern, leave and rejoin.
- Plate Gate scaling past 255 members, ORMap-of-CRDTs mechanics,
  the C1 vs C1-2 channel-record story.

Read this when working on governance, joins, or scaling past one
segment.

## Cross-references

- [`overview.md`](overview.md) — system stack and data-flow diagrams.
- [`crates.md`](crates.md) + [`crates-tier-detail.md`](crates-tier-detail.md) — the 33 workspace crates and their tier hierarchy.
- [`services-pattern.md`](services-pattern.md) — the runtime / adapter / pure-logic split used by every Tauri-side community surface.
- [`event-dispatch.md`](event-dispatch.md) — how `CommunityEvent`s reach the frontend.
- [`data-layer.md`](data-layer.md) — SQLite tables that back the community surfaces.
- [`../security/overview.md`](../security/overview.md) — five-layer encryption stack, identity, threat model.
- [`../protocol/overview.md`](../protocol/overview.md) — wire formats, Veilid primitives, DHT record layouts.
- [`../decisions/0003-flat-smpl-governance.md`](../decisions/0003-flat-smpl-governance.md) — ADR for the no-coordinator decision.

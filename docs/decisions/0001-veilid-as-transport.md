# 0001 — Adopt Veilid as the sole transport substrate

- **Status:** Accepted
- **Date:** 2026-04 (initial); reconfirmed 2026-05 in v2.0 architecture work

## Context and problem statement

Rekindle is a 1:1 reimplementation of the classic Xfire client. The
original Xfire was a client-server system with central infrastructure;
Rekindle reimagines it without any central server. We need a transport
that gives us:

- **Peer addressing** without DNS, IP, or a central directory.
- **NAT traversal** that works for residential users behind symmetric
  NAT.
- **Sender + receiver anonymity** comparable to Tor, with usable
  performance.
- **Distributed key-value storage** (DHT) so that profile records,
  presence, prekey bundles, and community state survive arbitrary peer
  churn.
- **Per-hop transport encryption** as a baseline.
- **A maintained ecosystem** — Rekindle is one project; we cannot
  shoulder the cost of building and operating a new P2P transport.

## Decision drivers

- **No central infrastructure.** Everything from NAT traversal to
  storage must be handled by the network itself.
- **Privacy posture.** Rekindle ships to vulnerable users; sender +
  receiver anonymity are load-bearing.
- **Audit-defensibility.** The transport must be a standard external
  thing, not a homebrew protocol that reviewers have to evaluate from
  scratch.
- **MIT-compatible licensing.** Rekindle is MIT; the transport must
  be a license we can distribute under.

## Considered options

### Option A — Veilid (selected)

Open-source decentralised application framework from the Veilid
project (Apache 2.0). Provides DHT, app_message / app_call datagrams,
private + safety routes, VICE NAT traversal, hop-by-hop transport
encryption.

### Option B — libp2p

Modular networking stack from Protocol Labs. DHT, pubsub, NAT
traversal via STUN/TURN/AutoRelay, transport encryption via Noise.

### Option C — Tor (onion services)

Mature anonymity network. Tor v3 onion services give us strong
sender/receiver anonymity. No native DHT abstraction; would need to
layer one.

### Option D — Custom protocol over UDP/QUIC

Build a transport tailored to our needs.

### Option E — I2P

Distinct anonymity network, similar properties to Tor, smaller user
base.

### Option F — Briar's own transport

Bluetooth + Tor-onion for nearby + remote. Tied to Briar's protocol.

## Decision outcome

**Chose Veilid.**

Veilid is the only option that delivers DHT, anonymous routing, NAT
traversal, and transport encryption in a single coherent package
without us having to glue them together. The semantics match our
needs almost exactly:

- Single-owner DFLT records and multi-writer SMPL records cover both
  "private profile data" and "shared community state" cleanly.
- `app_message` and `app_call` give us fire-and-forget and
  request-response datagrams — the two primitives needed for
  three-path delivery.
- Safety routes (sender anonymity, configurable hop count) and
  private routes (receiver anonymity) compose to give Tor-equivalent
  anonymity with much better latency, because Veilid has its own
  routing optimisation.
- VICE handles NAT traversal without us deploying STUN or TURN.
- Apache 2.0 license is MIT-compatible.

## Consequences

**Positive.**

- Communities inherit Veilid's global NAT-traversal and relay
  population for free. No per-community infrastructure cost.
- The transport is a single mature dependency rather than a stack of
  glue between libp2p / NAT / Tor / DHT layers.
- Veilid's hop-by-hop transport encryption gives us a baseline before
  application-layer encryption kicks in.
- Veilid is open source under Apache 2.0; we can maintain a fork if
  upstream stalls.

**Negative.**

- **Veilid is young.** veilid-core 0.5.x is the version we depend on;
  the ecosystem is small and we are likely the largest community-chat
  consumer. We accept this risk because the alternative (building our
  own equivalent) is worse.
- **Some primitives have constraints we work around** —
  `app_message` is small-payload (~32 KB cap), DHT records have
  practical writer limits (~255 SMPL members), `watch_dht_values` is
  unreliable enough that we add `inspect_dht_record` polling as a
  fallback. These constraints shape major design decisions
  (chunked file delivery, plate-gate scaling, three-path delivery).
- **Crate-level recursion-limit pressure.** Veilid's deeply nested
  futures need `#![recursion_limit = "512"]` in any crate that holds a
  long-lived future spanning Veilid I/O.
- **rusqlite version pinning.** Veilid depends on a specific rusqlite
  version; deviating would create libsqlite3-sys conflicts.

**Boundaries.**

- The Veilid integration must be confined to a small set of crates:
  `rekindle-protocol` (desktop) and `rekindle-transport` (daemon
  track) are the only `veilid_core` importers. This isolates our
  exposure to upstream API churn.
- We document the constraint that Veilid does **not** end-to-end
  encrypt — every Veilid relay sees plaintext. We layer Signal
  Protocol (1:1) and MEK (community channels) on top.

## Pros and cons of the options

### Veilid (chosen)

- **+** Anonymous routing built-in.
- **+** DHT built-in.
- **+** NAT traversal built-in.
- **+** Apache 2.0.
- **−** Young ecosystem; we are an early consumer.
- **−** Practical primitive constraints shape our design.

### libp2p

- **+** Mature, large ecosystem.
- **+** Modular — pick your own DHT, transport, encryption.
- **−** **Modular = glue.** We would assemble our own STUN/TURN, our
  own anonymity layer (libp2p has no first-class onion routing), our
  own transport encryption. Each glue point is a security review and
  a long-term maintenance commitment.
- **−** No first-class anonymity. AutoRelay etc. don't give Tor-class
  sender anonymity.

### Tor

- **+** Excellent anonymity properties.
- **−** No native DHT; we would need a separate distributed-storage
  primitive layered on top.
- **−** Latency is high for chat; voice is impractical without
  bypassing the Tor circuit.
- **−** Tor-network capacity is finite and shared with many other
  uses.

### Custom UDP/QUIC

- **−** Building this would dwarf the rest of the project. Out of
  scope.

### I2P

- **+** Anonymity properties similar to Tor.
- **−** Smaller user base, weaker tooling, less community review.
- **−** Same "no native DHT" problem as Tor.

### Briar's transport

- **+** Designed for messaging.
- **−** Tied to Briar's protocol; not extractable as a general
  substrate.

## More information

- [Veilid developer book](https://veilid.gitlab.io/developer-book/)
- [Veilid project on GitLab](https://gitlab.com/veilid/veilid)
- [`../architecture/overview.md`](../architecture/overview.md) — system layer stack
- [`../protocol/overview.md`](../protocol/overview.md) — wire protocol
- [`../security/privacy-properties.md`](../security/privacy-properties.md) — what Veilid gives vs. what Rekindle adds

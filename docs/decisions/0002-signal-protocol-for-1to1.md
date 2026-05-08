# 0002 — Use the Signal Protocol for 1:1 friend messaging

- **Status:** Accepted
- **Date:** 2026-04

## Context and problem statement

Rekindle's friend messaging needs to be end-to-end encrypted with
forward and backward secrecy. Two friends must be able to exchange
messages where:

1. A long-term key compromise does not retrospectively decrypt past
   messages.
2. A single message-key compromise does not affect future messages
   beyond the next ratchet step.
3. The session bootstraps from public-key material that can be
   published in advance (because peers may be offline when the other
   peer initiates).

We needed to choose between standard message-layer protocols.

## Decision drivers

- **Forward + backward secrecy** for 1:1 chat.
- **Asynchronous initiation.** Friends can be offline; the protocol
  must work against pre-published key material.
- **Mature, well-audited.** This is the most security-sensitive
  surface in the app. New / experimental protocols are excluded.
- **No central server.** Anything that requires a Signal-server
  equivalent is out.

## Considered options

### Option A — Signal Protocol (selected)

X3DH (Extended Triple Diffie-Hellman) for session establishment
plus the Double Ratchet for ongoing per-message keys. Operates
against published prekey bundles.

### Option B — Olm / Megolm (Matrix)

Olm is essentially the Signal Double Ratchet. Megolm is the group
extension.

### Option C — MLS (Messaging Layer Security)

[RFC 9420](https://datatracker.ietf.org/doc/html/rfc9420). Modern,
designed for groups, formal-method-friendly.

### Option D — Custom AKE on Curve25519

Build our own.

### Option E — OTR / OTRv4

Off-the-Record protocol.

## Decision outcome

**Chose the Signal Protocol.**

It is the protocol the security community has trusted for over a
decade. Every relevant property — forward secrecy, asynchronous
initiation, post-compromise recovery via the DH ratchet — is
delivered. Implementations exist in Rust (`libsignal`-derived) that
we can adapt.

X3DH operates against prekey bundles that we publish in the user's
DHT profile record. A friend who wants to start a session reads the
prekey bundle from the DHT, runs X3DH locally, and sends the first
message. The recipient's published prekey bundle is signed by their
identity key, so the bundle cannot be forged.

## Consequences

**Positive.**

- Per-message forward secrecy via the symmetric ratchet.
- Per-DH-step backward secrecy via the Diffie-Hellman ratchet —
  recovers from a single message-key compromise on the next round
  trip.
- Asynchronous initiation against DHT-published prekey bundles. No
  online ceremony required.
- Battle-tested. Independently audited. Known limits and known
  guarantees.
- Compatible vocabulary with the rest of the messaging-security
  ecosystem; reviewers do not need to learn a new protocol.

**Negative.**

- **Per-peer session state.** Each friend has a Signal session
  serialised in Stronghold. State accumulates with friend count;
  not a problem at human scale but worth noting.
- **PreKey replenishment is a real chore.** One-time prekeys are
  consumed on session establishment; we must replenish before they
  run out. This is on the roadmap as outstanding work.
- **No native multi-device.** Signal-Server uses sender keys for
  multi-device fan-out; we do not have a server. Each friend's
  device runs its own session — see
  [`../architecture/sync.md`](../architecture/sync.md) for cross-
  device sync's separate path.

## Pros and cons of the options

### Signal Protocol (chosen)

- **+** Industry-standard for 1:1 messaging.
- **+** Mature audit history.
- **+** Asynchronous initiation against published prekeys.
- **+** Self-healing ratchet.
- **−** Per-peer session state.
- **−** PreKey lifecycle requires careful replenishment.

### Olm / Megolm

- **+** Olm is essentially Signal's Double Ratchet.
- **+** Megolm is designed for groups.
- **−** Megolm assumes a Matrix-style room state — not directly
  applicable to our chiral-network community model.
- **−** Choosing Olm-instead-of-libsignal is a distinction without
  a difference. We picked the libsignal lineage because of
  audit-history and existing Rust tooling.

### MLS (RFC 9420)

- **+** Designed for groups; would be the natural answer for
  community channels.
- **+** Formal-method-friendly.
- **−** **Group-only.** For 1:1 chat, MLS is overkill; Signal
  Protocol fits better.
- **−** Ratchet-tree state machine **assumes a coordination point**
  for the group state. This conflicts with the chiral-network "no
  coordinator" property, which is why community channels use MEK +
  reader-validates governance instead. Revisit when a CRDT-friendly
  MLS variant matures.

### Custom AKE on Curve25519

- **−** Forbidden by policy. Custom crypto without years of audit
  history is not acceptable for the most security-sensitive surface.

### OTR / OTRv4

- **+** Forward secrecy, deniability properties.
- **−** Requires both parties online for session setup. Asynchronous
  initiation is mandatory for our use case.

## More information

- [Signal Protocol — X3DH](https://signal.org/docs/specifications/x3dh/)
- [Signal Protocol — Double Ratchet](https://signal.org/docs/specifications/doubleratchet/)
- [`libsignal`](https://github.com/signalapp/libsignal)
- [`../security/crypto-primitives.md`](../security/crypto-primitives.md) — primitive choices
- [`../architecture/communities.md`](../architecture/communities.md) — why community channels use MEK instead of MLS

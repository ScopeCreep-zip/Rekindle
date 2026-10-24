# 0023 — Mobile keeps the Host in-process permanently

- **Status:** Accepted (*Decided from research 2026-10-24:* plan §0 Q9, decided as its recommendation)
- **Date:** 2026-10-24
- **Amends:** [0010](0010-single-daemon-thin-frontends.md), for mobile only: its migration step 4 ("the GUI drops
  the in-process Host, connects to a spawned `rekindled`", `0010-single-daemon-thin-frontends.md:112-115`) applies
  to the desktop GUI and not to the Android and iOS apps.
- **Plan:** E5.6.1 (mobile target and push client) and F1 (GUI becomes a thin client) in
  `.claude/plans/standards-remediation/00-integration-plan.md` (v2.2), §0 Q9.
- **Related:** [0010](0010-single-daemon-thin-frontends.md) (one daemon, thin frontends),
  [0014](0014-veilid-boundary-is-transitive.md) (the Veilid boundary), [0020](0020-domain-logic-in-host-and-tier-crates.md)
  (domain logic in the Host and tier crates).

## Context and problem statement

ADR 0010 ends its strangler migration with the desktop GUI dropping the in-process `rekindle_node::Host` and
connecting to a spawned `rekindled` over the Noise IPC bus. E5.6.1 needs Android and iOS builds. An iOS app cannot
spawn and keep a helper process: the app sandbox is the only process the app gets. A mobile build that followed
step 4 would have no process to connect to.

## Decision drivers

ADR 0010's own drivers, applied to a phone:

- **One identity, one node per device.** One process on the phone hosts the node.
- **Privilege boundary.** On a phone the OS sandbox around the app is the boundary; there is no second process to
  move the keys into.
- **Fix once.** The mobile app runs the same `Host` and tier crates as the desktop and the daemon, so no domain logic
  is written twice.
- **No fallback paths.** At any moment exactly one process hosts the node.

## Decision outcome

Mobile is its own package that keeps `rekindle_node::Host` in-process (`TransportSource::Start`) permanently, as
the desktop GUI does between E5.7 and F1. F1 flips only `rekindle-desktop` to a bus client of `rekindled`; the
mobile package never links the IPC client for its own node and never spawns a daemon. Every user action still goes
through the Host's one intent gate (ADR 0019), and events reach the mobile webview through the same typed per-window
path (ADR 0013).

## Consequences

- E5.6.1 generates the Tauri 2 Android and iOS projects with the Host in-process, before and after F1.
- The mobile package keeps veilid-core in its process through `rekindle-node`, inside ADR 0014's transitive
  boundary; `rekindle-desktop`'s own exemption still ends at F1.
- ADR 0010's single-node rule holds per device: a phone is one device with one node, and pairing (E5.3) links it to
  the user's other devices as a separate device.
- The desktop's privilege split (keys out of the WebKit process) has no mobile analogue; the mobile threat model rests
  on the OS sandbox and the vault.

# Using Rekindle

Welcome. This directory holds the user-facing documentation: install
guides, walkthroughs of common tasks, and the FAQ.

| Document | Description |
|----------|-------------|
| [`getting-started.md`](getting-started.md) | Install, create an identity, add your first friend |
| [`install.md`](install.md) | Per-platform install instructions (Windows / macOS / Linux) |
| [`how-to.md`](how-to.md) | Walkthroughs for common tasks: add a friend, join a community, start a voice call, pair another device |
| [`faq.md`](faq.md) | Frequently asked questions and project-specific quirks |

If you want to **build the project from source** rather than install a
pre-built artifact, see [`../contributor/development.md`](../contributor/development.md).

If you want to **understand how Rekindle works under the hood**, start
with [`../../ARCHITECTURE.md`](../../ARCHITECTURE.md) at the repo root.

## A note on the user base

Rekindle is built primarily for users who care about privacy: gaming
friend groups who don't want their server-mate list scraped by
recommendation systems, activists, journalists, marginalized
communities, researchers. The product makes specific tradeoffs
(decentralised, no telemetry, no central server) that result in some
behaviours that are unusual compared to mainstream chat apps. The FAQ
addresses the most common ones.

## Status

Rekindle is **pre-1.0**. Behaviour, file formats, and protocols are
still evolving. We do not yet ship signed release artifacts; users
running a pre-release build are accepting that the schema and wire
format may change between updates. The first tagged release will mark
the start of backward-compatibility commitments.

See [`../roadmap.md`](../roadmap.md) for what's done and what's still
in progress.

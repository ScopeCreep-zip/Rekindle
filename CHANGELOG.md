# Changelog

All notable changes to Rekindle will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it reaches its first tagged release.

## [Unreleased]

Rekindle is pre-release. Day-to-day changes land on long-lived feature
branches (`codex/communities-*` for the v2.0 community migration; `main`
for the rest). The first tagged release will close out this section and
populate a versioned heading below.

### In active development

- **Communities v2.0 migration** — flat SMPL governance replacing the v1.0
  rotating-coordinator model. See
  [`docs/architecture/communities.md`](docs/architecture/communities.md)
  and [`docs/roadmap.md`](docs/roadmap.md).
- **Daemon + CLI track** — `rekindle-node` IPC bus, `rekindle-cli` TUI,
  `rekindle-transport` as the sole Veilid boundary. See
  [`docs/architecture/crates.md`](docs/architecture/crates.md).
- **Cross-segment channel records** (Plate Gates C1-2) — lazy per-segment
  channel records and cross-segment MEK distribution.

### Substantially complete

- 1:1 messaging (Signal Protocol over Veilid `app_message`).
- Friend management, friend groups, invites, blocking.
- Voice channels (cpal + Opus + RNNoise + AEC3 + jitter buffer + mixer).
- Cross-platform game detection (Linux, macOS, Windows process scanners).
- Custom titlebar / frameless transparent windows in the classic Xfire
  visual style.
- DMs and group DMs with X25519-derived MEK.
- Cross-device sync via personal DFLT record.
- Strand Relay forwarding for stale-route bridging.
- Mobile push relay (3-tier escalation: foreground / background fetch /
  opt-in relay).

### Known gaps

- PreKey rotation and one-time prekey replenishment.
- Game time tracking (elapsed, persisted in SQLite).
- Connection quality monitoring and display.
- File sharing UI polish, auto-update wiring, screen-share, overlay.

[Unreleased]: https://github.com/ScopeCreep-zip/Rekindle/commits/main

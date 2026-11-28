# Contributor Documentation

Onboarding for new contributors. Start with
[`../../CONTRIBUTING.md`](../../CONTRIBUTING.md) at the repo root for
the high-level overview (PR process, where to file issues, code of
conduct), then come back here for the technical detail.

| Document | Description |
|----------|-------------|
| [`development.md`](development.md) | Dev environment setup (Nix flake, manual setup), build commands, dependency overview |
| [`testing.md`](testing.md) | Test strategy: Rust unit/integration, property tests, netsim, the real-app suite (`pnpm test:app`) |
| [`style-guide.md`](style-guide.md) | Rust + TypeScript + Tailwind conventions; what the workspace lints enforce |
| [`release-process.md`](release-process.md) | Tagging, building, distributing, signing, post-release tasks |

For architectural context see [`../architecture/`](../architecture/),
the protocol details see [`../protocol/`](../protocol/), and the
security model see [`../security/`](../security/). The
[`../decisions/`](../decisions/) directory holds the ADRs that
explain *why* the system is shaped the way it is.

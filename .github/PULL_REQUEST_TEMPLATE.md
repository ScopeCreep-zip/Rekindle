<!--
Thanks for contributing to Rekindle! Please fill out the sections below.
If a section does not apply, write "n/a" instead of deleting it.
-->

## Summary

<!-- One or two sentences describing what this PR changes and why. -->

## Type of change

- [ ] Bug fix (non-breaking)
- [ ] New feature (non-breaking)
- [ ] Breaking change (existing API, schema, or wire format affected)
- [ ] Documentation only
- [ ] Refactor / cleanup (no behavior change)
- [ ] Build / CI / tooling

## Subsystem(s) touched

<!-- Tick all that apply. -->

- [ ] 1:1 messaging
- [ ] Communities
- [ ] DMs / group DMs
- [ ] Voice / video
- [ ] Game detection
- [ ] File sharing
- [ ] Cross-device sync
- [ ] Strand Relay / Push Relay
- [ ] Frontend / UI
- [ ] Tauri shell / window management
- [ ] Daemon / CLI
- [ ] Documentation only

## How was this tested?

<!-- Describe the testing you did. Examples:
     - "Added unit tests in rekindle-governance::merge."
     - "Ran cargo test --workspace + pnpm test:e2e."
     - "Verified by hand: opened two clients, created a community,
        confirmed both saw the new channel within 2 seconds."
-->

## Security / privacy review

<!-- If your change touches identity, encryption, transport, presence,
     MEK distribution, invite handling, or any always-on background
     behavior, please answer below. Otherwise write "n/a". -->

- Does this change introduce or remove any cryptographic primitive? <!-- yes/no, explain -->
- Does it change what data is sent over the wire (gossip, DHT, app_call)? <!-- yes/no, explain -->
- Does it change what data is persisted (SQLite, Stronghold, Store)? <!-- yes/no, explain -->
- Could a malicious peer abuse the new code path? <!-- yes/no, explain -->

## AI assistance

<!-- See docs/contributor/ai-assisted-contributions.md for the full policy. -->

- [ ] If any commit was AI-assisted, the commit message includes an
      `Assisted-by:` trailer naming the tool and version.
- [ ] No fabricated dependencies — every new package was verified on
      its registry page (crates.io / npmjs.com), checked for
      maintainer history, and run through `cargo audit` /
      `cargo deny` / `pnpm audit` locally.
- [ ] No new `#[allow(...)]` directives without an inline `reason = "…"`
      explaining why the lint is being silenced.
- [ ] No new tier-boundary violations (frontend imports of
      `@tauri-apps/api`, business logic in components, crypto outside
      `rekindle-secrets`, Veilid outside `rekindle-transport` /
      `rekindle-protocol`).
- [ ] AI-generated tests exercise real behaviour (no
      `expect(x).toBeDefined()` tautologies).

## Checklist

- [ ] `cargo test --workspace` passes locally.
- [ ] `cargo clippy --workspace -- -D warnings` is clean.
- [ ] `cargo fmt --all -- --check` is clean.
- [ ] `pnpm test:all` passes (if frontend or IPC was touched).
- [ ] `lefthook run pre-commit --all` is clean (covers the rest).
- [ ] Relevant docs in `/docs` updated.
- [ ] If this is a breaking schema change, `SCHEMA_VERSION` was bumped in `db.rs`.
- [ ] No `#[allow(dead_code)]`, `dbg!`, `todo!()`, `unimplemented!()` introduced.
- [ ] No legacy compatibility shims added (project is pre-release).

## Related issues / context

<!-- Link issues, RFCs, or prior PRs. e.g. "Closes #123" or "Follows up on #45". -->

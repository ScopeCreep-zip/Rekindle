# Reproducible Builds

Reproducible builds let any user verify that a downloaded artefact
exactly matches the source code. Two independent builds of the same
commit produce byte-identical output — so an attacker who tampered
with the build infrastructure cannot substitute a malicious binary
without being detected.

This is a load-bearing property for security-conscious users:
download the artefact, run the build yourself, compare hashes, and
you have cryptographic certainty (modulo your trust in the source
tree).

## Status

**Not yet end-to-end reproducible.** This document describes the
plan, the pieces in place, and the work still required.

## Pieces in place

| Piece | Where |
|-------|-------|
| Pinned Rust toolchain | [`../../rust-toolchain.toml`](../../rust-toolchain.toml) — `channel = "1.92.0"` |
| Pinned Cargo lockfile | [`../../Cargo.lock`](../../Cargo.lock) committed |
| Pinned npm/pnpm lockfile | [`../../pnpm-lock.yaml`](../../pnpm-lock.yaml) committed |
| Pinned Nix flake | [`../../flake.lock`](../../flake.lock) committed; `flake.nix` provides the hermetic dev shell |
| Deterministic Cap'n Proto codegen | `capnp` compiler invoked from `build.rs` with checked-in schemas |
| `RUSTFLAGS = "-D warnings"` in CI | [`../../.github/workflows/ci.yml`](../../.github/workflows/ci.yml) |
| `LF` line endings everywhere | [`../../.gitattributes`](../../.gitattributes) |
| `.editorconfig` whitespace rules | [`../../.editorconfig`](../../.editorconfig) |

## Pieces still to land

### 1. `SOURCE_DATE_EPOCH` propagation

The `SOURCE_DATE_EPOCH` environment variable
(<https://reproducible-builds.org/specs/source-date-epoch/>) sets the
"now" timestamp every build tool should use for embedded timestamps.
We need to:

- Set `SOURCE_DATE_EPOCH` from the git commit timestamp in CI.
- Verify Tauri's bundler honours it for NSIS / DMG / AppImage / `.deb`.
- File issues upstream where it doesn't.

### 2. Tauri bundler determinism

The Tauri bundler embeds metadata into installer / package files.
Several fields default to non-deterministic values:

- Build timestamp (file mtime in NSIS / DMG / DEB).
- Build host hostname (some bundles include it).
- Random temp-directory paths during the bundle step.

Each needs to be either fixed via `SOURCE_DATE_EPOCH` /
`HOSTNAME=rekindle-build` / a dedicated temp dir, or patched
upstream.

### 3. Embedded asset hashes

The icon set, font files, and other static assets are embedded into
the bundle. As long as the source files don't change, this is fine —
but the *order* of embedding matters for some bundle formats. We need
to verify that the asset embedding order is deterministic.

### 4. CI verification

Two independent CI runs of the same commit must produce byte-
identical artefacts. The verification workflow:

1. CI build A on a clean runner.
2. CI build B on a different clean runner (or same runner, fresh
   checkout).
3. `sha256sum` and `b3sum` every artefact from both builds.
4. Compare. Any mismatch is an artefact-level bug.

### 5. Per-platform reproducibility

Each target OS has its own bundle format and its own determinism
issues:

- **Linux (AppImage, .deb)** — most reproducible, mature
  reproducible-builds tooling.
- **macOS (DMG)** — code-signing introduces non-determinism;
  notarisation timestamps are signed by Apple. The signature itself
  is therefore non-reproducible. We can verify the *unsigned*
  artefact reproduces, then check the signature is valid.
- **Windows (NSIS)** — Authenticode signature has the same
  non-reproducibility as macOS; verify unsigned, then check signature.

### 6. SBOM cross-verification

The SBOM workflow generates a CycloneDX SBOM on every release. A
verifying user should be able to:

1. Reproduce the build locally from the source tree.
2. Generate the same SBOM with `cargo cyclonedx --format json`.
3. Compare against the released SBOM.

Any mismatch indicates either build-environment drift or supply-
chain tampering.

## How a user verifies a release (today)

Even without full reproducibility, users can do partial verification:

1. **Build from source** following
   [`../user/install.md`](../user/install.md) "Building from source".
2. **Generate the SBOM** locally:
   ```sh
   cargo install cargo-cyclonedx --locked
   cargo cyclonedx --format json
   ```
3. **Compare the dependency graph** against the released SBOM. The
   CycloneDX file lists every crate version; mismatches are easy to
   spot.
4. **Compare the SHA-256 of the artefact** to the value published
   alongside the release (once we publish them).

The full byte-identical comparison is the goal of this document.

## How a user verifies a release (post-reproducibility)

Once items 1–6 above land:

```sh
# Clone at the tag
git clone --branch vX.Y.Z https://github.com/ScopeCreep-zip/Rekindle
cd Rekindle

# Build with SOURCE_DATE_EPOCH set
export SOURCE_DATE_EPOCH=$(git log -1 --format=%ct)
nix develop .#frontend
pnpm install --frozen-lockfile
pnpm tauri build

# Hash the artefact
sha256sum src-tauri/target/release/bundle/<os>/Rekindle*

# Compare against the published hash
curl -fsSL https://github.com/ScopeCreep-zip/Rekindle/releases/download/vX.Y.Z/SHA256SUMS

# Optionally verify the signature (post-v0.1.0)
gpg --verify SHA256SUMS.asc SHA256SUMS
```

## Why this matters more than usual

Rekindle ships to vulnerable users who would be materially harmed by
a substituted binary. The threat model
([`threat-model.md`](threat-model.md)) explicitly tracks supply-chain
compromise (T6) as in-scope. Without reproducible builds, we are
asking users to trust whatever artefact lands on the GitHub Releases
page — a single point of failure in the publishing pipeline.

## References

- [Reproducible Builds project](https://reproducible-builds.org/)
- [`SOURCE_DATE_EPOCH` specification](https://reproducible-builds.org/specs/source-date-epoch/)
- [Tauri reproducible-builds tracking issue (upstream)](https://github.com/tauri-apps/tauri/) — link to specific issue when filed
- [`supply-chain-policy.md`](supply-chain-policy.md)
- [`../contributor/release-process.md`](../contributor/release-process.md)
- [`threat-model.md`](threat-model.md) §T6

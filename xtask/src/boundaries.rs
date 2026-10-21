//! `check-boundaries`: the crate-import tier boundaries.

use std::path::Path;

use anyhow::{anyhow, Result};

// ────────────────────────────────────────────────────────────────
// check-boundaries
// ────────────────────────────────────────────────────────────────
//
// Tier-2 invariant: only `rekindle-secrets` may import these crypto
// crates directly. Tier-3+ consumers must go through `rekindle-secrets`.
const CRYPTO_CRATES: &[&str] = &[
    "ed25519-dalek",
    "x25519-dalek",
    "aes-gcm",
    "chacha20poly1305",
    // The Double Ratchet's ENCRYPT (DR rev. 4 §7.2: AES-256-CBC + HMAC).
    "aes",
    "cbc",
    // `rekindle_utils::random::ForkedRandom`'s generator.
    "chacha20",
    "hkdf",
    // Noise protocol framework (the IPC bus handshake and transport).
    "snow",
];
// The Veilid boundary is transitive linkage (ADR 0014): `veilid_boundary`
// walks the resolved graph, so a crate that reaches veilid-core through
// another crate fails as surely as one that names it.
// Crypto-allowed crates for now; this list will shrink as the cleanup
// sweep refactors crypto consumers to consume via rekindle-secrets.
const CRYPTO_ALLOWED: &[&str] = &[
    "rekindle-secrets",
    // The IPC bus is Noise IK end to end; `rekindle-ipc` owns the
    // handshake and transport state (plan C1). Tier 3.
    "rekindle-ipc",
    // ── Pending sweep — see ai-assisted-contributions.md §5 ──
    "rekindle-crypto",
    "rekindle-dm",
    "rekindle-calls",
    "rekindle-voice",
    "rekindle-node",
    "rekindle-transport",
    "rekindle-records",
    "rekindle-route",
    "rekindle-sync",
    "rekindle-protocol",
    // ── Decomposed-harvest plan (memoized-dazzling-torvalds.md) ──
    // Phase 2: SQLCipher + per-entry AES-256-GCM vault layer.
    "rekindle-vault",
    // Phase 4: BLAKE3 keyed hash chain. (BLAKE3 is not in CRYPTO_CRATES,
    // but listing here keeps the intent explicit if future deps grow.)
    "rekindle-audit",
    // Phase 17: cascade MEK rotation — wraps MEKs via rekindle-crypto's
    // mek_distribution helpers.
    "rekindle-mek-rotation",
    // Phase 18: community lifecycle orchestration (origin / bootstrap /
    // join / segments / apply) — uses rekindle-crypto for pseudonym
    // derivation + governance entry signing.
    "rekindle-governance-runtime",
    // Phase 19: community channel messaging (send / receive / threads /
    // reactions / expressions / mentions) — uses rekindle-crypto for
    // MEK encrypt/decrypt + signed-envelope auth.
    "rekindle-channel",
];

pub(crate) fn check_boundaries(root: &Path) -> Result<()> {
    let crates_dir = root.join("crates");
    if !crates_dir.exists() {
        return Err(anyhow!("crates/ directory not found"));
    }

    let mut violations: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&crates_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let crate_name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| anyhow!("non-utf8 crate dir"))?
            .to_owned();
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.exists() {
            continue;
        }
        let toml = std::fs::read_to_string(&manifest)?;

        for crypto in CRYPTO_CRATES {
            if dep_present(&toml, crypto) && !CRYPTO_ALLOWED.contains(&crate_name.as_str()) {
                violations.push(format!(
                    "{crate_name}: imports `{crypto}` (Tier 2 crypto boundary — only via rekindle-secrets)"
                ));
            }
        }
    }
    violations.extend(crate::veilid_boundary::check_veilid_boundary(root)?);

    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Tier-boundary violations:");
    for v in &violations {
        eprintln!("  • {v}");
    }
    eprintln!(
        "\nSee docs/contributor/architecture-rules.md for the binding hierarchy.\n\
         Most existing violations are tracked in the cleanup sweep — see\n\
         docs/contributor/ai-assisted-contributions.md §5."
    );
    Err(anyhow!("{} boundary violation(s)", violations.len()))
}

fn dep_present(toml: &str, dep: &str) -> bool {
    // Crude but adequate: a `name = "..."` table-form entry or a
    // `name.workspace = true` line. Avoids pulling in toml/serde here
    // to keep xtask compile time cheap.
    let needles = [
        format!("\n{dep} = "),
        format!("\n{dep}.workspace"),
        format!("\n\"{dep}\" = "),
        format!("name = \"{dep}\""),
    ];
    needles.iter().any(|n| toml.contains(n.as_str()))
}

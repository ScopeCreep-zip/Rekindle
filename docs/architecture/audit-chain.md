# Audit Chain

`rekindle-audit` is a BLAKE3-keyed hash chain for tamper-evident
audit logging. Every audit entry carries `prev_mac + mac` such that
tampering with any byte of any entry's `payload_json` invalidates
every entry from that cursor forward.

This document covers the crate, the Tauri-side persistence and
verification surfaces, and the rationale for the threat model.

## Why a keyed hash chain, not a Merkle tree or Ed25519 signatures

- **Merkle trees** are great for batched verification (e.g., a
  blockchain block) but require either a global root that everyone
  watches or a separate publication channel. Rekindle has neither:
  each peer's audit log is private to that peer and verifies locally.
- **Ed25519 signatures** would force every audit append to do a
  signing operation. The audit chain runs on the same thread as the
  Tauri command that triggered it (community-create, channel-create,
  role-edit, member-ban, MEK-rotate, vault-passphrase-change, …), so
  a 200-microsecond sign per append would burn user-visible latency
  on every administrative action.
- **BLAKE3-keyed hash** is one operation, sub-microsecond, and
  produces the same tamper-evident chain semantics for the local-
  audit threat model.

The trade is: a keyed hash chain protects against **tampering** but
not against **forging by anyone with the key**. That is acceptable
because the same vault holds the identity secret being "audited" —
loss of the vault loses the chain; theft of the vault gains
MAC-forgery capability, which is no worse than what the attacker
already has by virtue of holding the identity secret.

## Chain construction

```
mac_i = BLAKE3-keyed(mac_key, prev_mac_i || cursor_le_i || payload_json_i)
prev_mac_{i+1} = mac_i
```

- `mac_key` is a 32-byte secret generated on first vault unlock and
  stored at `("audit", "mac_key")` in the vault.
- `prev_mac_0` is a 32-byte zero seed.
- `cursor_le` is the entry's monotonic `u64` cursor, little-endian.
- `payload_json` is the entry's payload, serialized to canonical
  JSON before MAC computation.

The crate exposes:

```rust
pub struct AuditChain { /* keyed BLAKE3 hasher + prev_mac */ }

impl AuditChain {
    pub fn new(mac_key: &[u8; 32]) -> Self;
    pub fn resume_from(mac_key: &[u8; 32], tail_anchor: [u8; MAC_LEN]) -> Self;
    pub fn append(&mut self, cursor: u64, payload: &serde_json::Value) -> AuditEntry;
    pub fn verify(&self, entries: &[AuditEntry]) -> Result<(), VerifyError>;
}

pub struct AuditEntry {
    pub cursor: u64,
    pub kind: AuditKind,
    pub payload_json: serde_json::Value,
    pub prev_mac: [u8; MAC_LEN],
    pub mac: [u8; MAC_LEN],
}

pub enum AuditKind { /* CommunityCreate, ChannelCreate, MemberBan, … */ }

pub struct AuditRecord { /* AppState handle wiring */ }

pub const MAC_LEN: usize = 32;
```

Single module `chain` plus re-exports. Zero dependencies beyond
`blake3`, `serde`, and `zeroize` (the `mac_key` zeroises on
`AuditChain` drop).

## Tauri-side persistence (`src-tauri/src/audit_repo/`)

The audit chain object is held on `AppState.audit_chain:
Mutex<Option<AuditChain>>`. The repo wraps SQLite persistence and
the vault round-trip:

| Module | Responsibility |
|---|---|
| `audit_repo/mod.rs` | Public API: `restore_chain`, `append_entry`, `verify_all`, `export_all` |
| `audit_repo/chain.rs` | Resume the chain from the most-recent stored `tail_anchor` in the vault |
| `audit_repo/store.rs` | SQLite reads / writes against `audit_entries` |
| `audit_repo/tests.rs` | Chain integrity test fixtures |

On every append, the repo:

1. Acquires the chain mutex.
2. Calls `chain.append(cursor, payload)`.
3. Writes the resulting `AuditEntry` to `audit_entries` via
   `db_helpers::db_call`.
4. Persists the new tail `mac` to the vault under
   `("audit", "tail_anchor")` so a process restart can resume
   without re-walking the chain.

`audit_view.rs` is the read-side facade: paginated queries, kind
filters, and the export payload format used by `audit_export`.

## Audit kinds

Audit kinds the chain currently tracks:

- Community: `CommunityCreate`, `ChannelCreate`, `ChannelDelete`,
  `RoleCreate`, `RoleEdit`, `RoleDelete`, `MemberBan`, `MemberUnban`,
  `MemberTimeout`, `MekRotate`, `SegmentAdded`,
  `OnboardingComplete`.
- Account: `IdentityCreate`, `LoginSucceeded`, `LoginFailed`,
  `LogoutClean`, `VaultPassphraseChanged`, `VaultExported`,
  `DeviceLinked`, `DeviceUnlinked`.
- Cryptography: `SignalSessionReset`, `PrekeyRotated` (when wired).

The `AuditKind` enum is the source of truth. Adding a new kind is a
SQL-schema-bump-free change since `audit_entries` stores
`payload_json` opaquely.

## Verification surfaces

Two Tauri commands expose verification to the UI:

- `commands/community/audit::get_audit_log` — paginated read of the
  current community's audit entries. Implicitly verifies the chain
  by recomputing each entry's MAC against the stored `prev_mac` and
  rejecting any mismatch (returns `Error("audit chain broken at
  cursor N")`).
- `commands/vault::audit_verify` — manual full-chain walk from
  cursor 0 to the current tail. Used by the Settings → Security
  surface and triggered on suspicion of tampering.

A third command `commands/vault::audit_export` produces a signed
export bundle (the entries plus the current tail MAC, sealed under a
fresh Ed25519 signature with the identity key) for off-device backup
or compliance hand-off.

## Tail-anchor invariant

The vault entry `("audit", "tail_anchor")` is the security
critical invariant. If it is missing or mismatched on startup, the
chain refuses to extend — the user is prompted to verify the chain
manually (re-walk from cursor 0) and either accept the divergence
(rare-cause: backup restore) or reject and rebuild from the most
recent verified cursor (more common: corruption).

The `audit_chain::resume_from` function is the only legitimate path
to load a non-zero `prev_mac` on construction. Tests pin this
contract — there is no escape hatch that lets a caller skip the
resume step.

## Why this lives in Tier 2

Tier 2 is the cryptographic and persistent boundary. `rekindle-audit`
fits because:

- It owns key material (the MAC key).
- It owns the chain state (tail anchor).
- Its consumers (`audit_repo`, `commands/community/audit`,
  `commands/vault`) only call back into the chain through the
  `AppendOnly` / `Verify` surfaces — they never touch the MAC key
  themselves.

The Tier-2 placement is enforced by the CI tier-violation lint
(`grep` for `blake3` in higher tiers — only Tier 2 may import it).

See [`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md)
for the tier-bump argument that placed `rekindle-audit`, `rekindle-vault`,
and `rekindle-secrets` together at Tier 2.

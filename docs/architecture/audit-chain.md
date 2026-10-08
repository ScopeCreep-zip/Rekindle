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
pub struct AuditChain { /* zeroizing key, last_mac, last_cursor */ }

impl AuditChain {
    pub fn open(key: Zeroizing<[u8; 32]>, last_mac: [u8; MAC_LEN], last_cursor: u64) -> Self;
    pub fn append(&mut self, record: AuditRecord) -> Result<AuditEntry, serde_json::Error>;
    pub fn verify(&self, entries: &[AuditEntry]) -> Result<(), VerifyError>;
}

pub struct AuditRecord { pub at_ms: i64, pub actor_pub: String, pub kind: AuditKind, pub payload: serde_json::Value }
pub struct AuditEntry { pub cursor: u64, pub prev_mac: [u8; MAC_LEN], pub mac: [u8; MAC_LEN], pub record: AuditRecord }
pub enum AuditKind { FriendAdded, FriendRemoved, ChannelJoined, ChannelLeft, IdentityRotated, VaultUnlocked }

pub enum TailCheck { Unanchored, Clean, CatchUp, Tampered { anchor: Tail } } // tail::TailCheck::of(anchor, stored)
pub const MAC_LEN: usize = 32;
```

Modules `chain` and `tail`. Dependencies: `blake3`, `hex`, `serde`,
`serde_json`, `thiserror`, `tracing`, `zeroize` (the key zeroises on drop).

## Tauri-side persistence (`src-tauri/src/audit_repo/`)

The audit chain object is held on `AppState.audit_chain:
Mutex<Option<AuditChain>>`. The repo wraps SQLite persistence and
the vault round-trip:

| Module | Responsibility |
|---|---|
| `src-tauri/src/audit_repo/chain.rs` | `append_async`, `verify_async`, `restore_chain`: the desktop's chain state, events and toasts |
| `rekindle_db::repo::audit` | SQLite reads / writes against `audit_entries`, with tests against the real schema (tamper, truncation, owner isolation) |
| `rekindle_audit::TailCheck` | The vault tail-anchor rule `restore_chain` applies (clean, catch-up, tampered) |
| `rekindle_vault::typed::audit` | The MAC key and tail anchor in the vault |

On every append, the repo:

1. Acquires the chain mutex.
2. Calls `chain.append(cursor, payload)`.
3. Writes the resulting `AuditEntry` to `audit_entries` via
   `db_helpers::db_call`.
4. Persists the new tail `(cursor, mac)` to the vault under
   `("audit", "tail")` so a process restart can resume
   without re-walking the chain.

`audit_view.rs` is the read-side facade: paginated queries, kind
filters, and the export payload format used by `audit_export`.

## Audit kinds

`AuditKind` is the source of truth: `FriendAdded`, `FriendRemoved`,
`ChannelJoined`, `ChannelLeft`, `IdentityRotated`, `VaultUnlocked`. Adding
a kind needs no schema bump: `audit_entries` stores the record as opaque
JSON.

## Verification surfaces

- `audit_verify` (`commands/auth.rs`) re-MACs every entry from cursor 0
  (`verify_async`). A break emits a `SystemAlert` toast naming the cursor.
  Login runs the same verify after unlocking the vault.
- `audit_export` returns the stored entries after a cursor, unsigned;
  anyone with the MAC key can re-verify them.

The community audit log (`get_audit_log`) is a different thing: the
signed governance audit entries read from the community's DHT records,
not this chain.

## Tail-anchor invariant

Every append writes the new `(cursor, mac)` to the vault under
`("audit", "tail")` as well as the row to SQLite. Dropping trailing rows
leaves a chain that still verifies, only shorter; the anchor is what
shows it. `restore_chain` compares the two at unlock with
`rekindle_audit::TailCheck`:

| Anchor vs stored tail | Result |
|---|---|
| no anchor (new identity) | `Unanchored`: resume from the stored tail |
| equal | `Clean` |
| anchor behind | `CatchUp`: an append's vault write was lost (logout or crash between the two writes); resume from the stored tail, and the boot-time verify re-MACs the gap |
| anchor ahead, or same cursor with another MAC | `Tampered`: resume from the anchor, the last entry known good, and raise `AuditChainBroken` plus a `SystemAlert` |

The vault is a separate, encrypted file, so whoever can edit the
database cannot move the anchor with it.

## Why this lives in Tier 2

Tier 2 is the cryptographic and persistent boundary. `rekindle-audit`
fits because:

- It owns key material (the MAC key).
- It owns the chain state and the tail-anchor rule.
- Its consumer (`audit_repo`) only calls `append` / `verify` and
  `TailCheck`; it never computes a MAC itself.

The Tier-2 placement is enforced by the CI tier-violation lint
(`grep` for `blake3` in higher tiers — only Tier 2 may import it).

See [`decisions/0009-crate-harvest-tiers.md`](../decisions/0009-crate-harvest-tiers.md)
for the tier-bump argument that placed `rekindle-audit`, `rekindle-vault`,
and `rekindle-secrets` together at Tier 2.

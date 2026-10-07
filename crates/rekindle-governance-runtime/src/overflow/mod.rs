//! M10.4 — `GovernanceOverflow`: per-author governance subkey spill.
//!
//! The universal Q-pid SMPL governance schema gives each author exactly one
//! subkey (`m_cnt: 1`), capped at ~4112 B. A prolific author (typically the
//! genesis admin authoring community-meta + every role + every channel +
//! invites) eventually fills it. Rather than drop entries, the author spills
//! the excess into a chain of member-owned **overflow records** and publishes a
//! pointer on the signed subkey header (`GovernanceSubkeyPayload::overflow_next`,
//! architecture §"Follow GovernanceOverflow pointers" line 1609; `overflow_next`
//! header line 305).
//!
//! Convergence is preserved because [`rekindle_governance::merge::merge`]
//! flattens every `(author, entries)` pair and re-sorts by Lamport: splitting
//! one author's log across `[primary subkey, overflow record 0, …]` is
//! invisible to the merge **iff every reader reassembles the full set first**.
//! [`read_governance_with_overflow`] does exactly that on the read path;
//! [`read_my_chain`] does it on the write path before re-compaction.
//!
//! ## Overflow-record lifecycle (grounded in veilid-core 0.5.2)
//!
//! An overflow record is a single-owner `DFLT(1)` record whose owner keypair is
//! **derived** from the identity secret
//! ([`derive::derive_governance_overflow_keypair`]) — that grants write
//! authority on any device with no persisted keypair. The record **key** is NOT
//! re-derivable: `create_dht_record` always mixes in a random encryption key
//! (`storage_manager/create_record.rs:87`) and *refuses to re-create* a record
//! for an existing owner+schema (`routing_context.rs:360`). So the key is
//! captured once at create and persisted in the DHT chain (`overflow_next`) +
//! the local Veilid store. The write path anchors a freshly-created key by
//! writing the primary subkey's pointer immediately, so a later device or retry
//! re-opens via the persisted key rather than re-creating. A `DFLT(1)` subkey
//! caps at 32768 B (`min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE/o_cnt)`), 8× a
//! governance subkey, so one overflow record absorbs ~30 KB of governance and
//! the record (and create) count stays at ~1 for any realistic author.
//!
//! Tier 7 helper — pure partitioning + deps-mediated DHT I/O, no Veilid types.

use std::collections::HashSet;

use rekindle_records::lease::LeaseId;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// SMPL per-subkey cap for a 255-slot governance record:
/// `min(MAX_SUBKEY_SIZE 32768, MAX_RECORD_DATA_SIZE 1_048_576 / 255)`.
const SMPL_SUBKEY_MAX_BYTES: usize = 1_048_576 / 255; // 4112

/// `DFLT(1)` per-subkey cap: `min(MAX_SUBKEY_SIZE 32768, MAX_RECORD_DATA_SIZE
/// 1_048_576 / o_cnt 1)` = 32768 (veilid-core `storage_manager/schema.rs:59`,
/// `EncryptedValueData::MAX_LEN`).
const DFLT_SUBKEY_MAX_BYTES: usize = 32_768;

/// Entry-bytes budget for page 0 — the primary SMPL governance subkey. Headroom
/// under [`SMPL_SUBKEY_MAX_BYTES`] for the JSON wrapper (author_pseudonym +
/// signature + `overflow_next` pointer), array framing, and the encryption
/// overhead veilid adds before its own cap check.
pub const PRIMARY_PAGE_BUDGET: usize = SMPL_SUBKEY_MAX_BYTES - 600; // 3512

/// Entry-bytes budget for overflow pages — `DFLT(1)` records. ~8× the primary,
/// so a single overflow record absorbs ~30 KB of governance and the chain
/// length (hence create count) stays near 1 for any real community.
pub const OVERFLOW_PAGE_BUDGET: usize = DFLT_SUBKEY_MAX_BYTES - 2_768; // 30000

/// Overflow records hold their single page in subkey 0 (`DFLT(1)`).
const OVERFLOW_SUBKEY: u32 = 0;

/// Hard cap on overflow-chain length, applied symmetrically on read (follow) and
/// write (build). At [`OVERFLOW_PAGE_BUDGET`] each page holds ~75 entries, so 64
/// pages ≈ 4800 entries for ONE author — far beyond any real community, while
/// bounding a malicious unbounded chain a reader would otherwise walk forever.
pub const MAX_OVERFLOW_PAGES: usize = 64;

/// The narrow DHT-bytes surface the overflow read/write helpers actually touch —
/// a strict subset of [`GovernanceRuntimeDeps`], blanket-implemented for every
/// deps impl so production callers pass their existing `deps` unchanged.
/// Segregating it keeps these helpers (and their tests) decoupled from the
/// 60-method composite trait: a test mock implements four methods, not sixty.
#[async_trait::async_trait]
pub trait OverflowIo: Send + Sync {
    /// Borrow a record from the host's pool (see
    /// [`GovernanceRuntimeDeps::acquire_record`]).
    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<LeaseId, GovernanceRuntimeError>;
    async fn release_record(&self, lease: LeaseId);
    async fn get_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    async fn set_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String;
    /// Whether the owning session is ending: the reader stops before its
    /// next call.
    fn stop_requested(&self) -> bool;
}

#[async_trait::async_trait]
impl<D: GovernanceRuntimeDeps> OverflowIo for D {
    async fn acquire_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<LeaseId, GovernanceRuntimeError> {
        GovernanceRuntimeDeps::acquire_record(self, record_key, writer).await
    }
    async fn release_record(&self, lease: LeaseId) {
        GovernanceRuntimeDeps::release_record(self, lease).await;
    }
    async fn get_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        GovernanceRuntimeDeps::get_dht_value(self, lease, subkey, force_refresh).await
    }
    async fn set_dht_value(
        &self,
        lease: LeaseId,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        GovernanceRuntimeDeps::set_dht_value(self, lease, subkey, value, writer).await
    }
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        GovernanceRuntimeDeps::format_writer_keypair(self, ed_public, ed_secret)
    }
    fn stop_requested(&self) -> bool {
        crate::join_gate::should_stop(self)
    }
}

/// Read one subkey of `record_key` on a borrow of its own: acquire, get,
/// release. While the community holds the record for its session the
/// acquire is a table hit.
pub(crate) async fn read_subkey<D: OverflowIo + ?Sized>(
    deps: &D,
    record_key: &str,
    subkey: u32,
    force_refresh: bool,
) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
    let lease = deps.acquire_record(record_key, None).await?;
    let value = deps.get_dht_value(lease, subkey, force_refresh).await;
    deps.release_record(lease).await;
    value
}

/// Greedily pack a compacted entry log into pages that each serialize within
/// budget. Page 0 (the primary SMPL subkey) uses `primary_budget`; pages ≥1
/// (overflow `DFLT(1)` records) use the larger `overflow_budget`. An entry that
/// does not fit the small primary page opens an overflow page instead, so the
/// primary may legitimately end up empty (everything spilled). Returns `Err`
/// with the offending size if a *single* entry exceeds `overflow_budget` — it
/// is unsplittable (a should-never-happen safety net; real entries are < ~500 B).
pub fn partition_into_pages(
    entries: Vec<GovernanceEntry>,
    primary_budget: usize,
    overflow_budget: usize,
) -> Result<Vec<Vec<GovernanceEntry>>, usize> {
    let mut pages: Vec<Vec<GovernanceEntry>> = vec![Vec::new()];
    let mut cur = 0usize;
    for entry in entries {
        let sz = serde_json::to_vec(&entry)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
        if sz > overflow_budget {
            return Err(sz);
        }
        // Advance pages (mutating `pages`/`cur` only — never `entry`) until the
        // current page can hold this entry, then push exactly once.
        loop {
            let on_primary = pages.len() == 1;
            let budget = if on_primary {
                primary_budget
            } else {
                overflow_budget
            };
            let last = pages.last().expect("pages always has a last page");
            let fits = if last.is_empty() {
                sz <= budget
            } else {
                cur + sz <= budget
            };
            if fits {
                break;
            }
            // Either the page is full, or the (empty) primary is too small for
            // this entry — in both cases open a fresh page (overflow budget).
            pages.push(Vec::new());
            cur = 0;
        }
        let page = pages.last_mut().expect("pages always has a last page");
        if page.is_empty() {
            cur = sz;
        } else {
            cur += sz;
        }
        page.push(entry);
    }
    Ok(pages)
}

/// Build and sign a `GovernanceSubkeyPayload` for one page, returning the struct
/// and its canonical JSON bytes. The `overflow_next` pointer is bound into the
/// signature (`signing_bytes`, the canonical `rekindle-gov-subkey-v1` domain).
pub fn build_signed_payload(
    entries: &[GovernanceEntry],
    overflow_next: Option<String>,
    signing_key: &SigningKey,
    author: &PseudonymKey,
) -> Result<(GovernanceSubkeyPayload, Vec<u8>), GovernanceRuntimeError> {
    let mut payload = GovernanceSubkeyPayload {
        author_pseudonym: author.clone(),
        entries: entries.to_vec(),
        overflow_next,
        signature: Vec::new(),
    };
    let sig = derive::sign_with_pseudonym(signing_key, &payload.signing_bytes());
    payload.signature = sig.to_vec();
    let bytes = serde_json::to_vec(&payload)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("serialize governance page: {e}")))?;
    Ok((payload, bytes))
}

/// Parse + W26-verify one subkey payload. Returns `None` (drop) if it fails to
/// deserialize, the author signature is invalid, or — when `expect_author` is
/// `Some` — the payload is authored by a different pseudonym (an overflow record
/// must be signed by the same author as the primary that pointed at it, else a
/// rogue member could redirect the chain at a record they control).
fn verify_payload(
    bytes: &[u8],
    expect_author: Option<&PseudonymKey>,
) -> Option<GovernanceSubkeyPayload> {
    let payload = serde_json::from_slice::<GovernanceSubkeyPayload>(bytes).ok()?;
    if let Some(expected) = expect_author {
        if payload.author_pseudonym != *expected {
            return None;
        }
    }
    let sig: [u8; 64] = payload.signature.as_slice().try_into().ok()?;
    derive::verify_pseudonym_signature(&payload.author_pseudonym.0, &payload.signing_bytes(), &sig)
        .ok()?;
    Some(payload)
}

/// This author's full logical entry log plus the ordered keys of the overflow
/// records currently in their chain (`overflow_keys[i]` is page `i + 1`).
#[derive(Debug)]
pub struct MyChain {
    pub entries: Vec<GovernanceEntry>,
    pub overflow_keys: Vec<String>,
}

/// Reassemble the writing author's entire entry log across the primary subkey +
/// every overflow record in their chain, and capture the existing overflow
/// record keys for reuse. W26-verifies each payload against `me` (our own
/// pseudonym). Cycle-guarded and depth-capped. Missing/unverifiable pages stop
/// the walk (the chain self-heals on the next successful write).
pub async fn read_my_chain<D: OverflowIo>(
    deps: &D,
    gov_key: &str,
    my_slot: u32,
    me: &PseudonymKey,
) -> Result<MyChain, GovernanceRuntimeError> {
    let mut entries = Vec::new();
    let mut overflow_keys = Vec::new();

    let primary_bytes = read_subkey(deps, gov_key, my_slot, false)
        .await?
        .filter(|b| !b.is_empty());
    let Some(primary_bytes) = primary_bytes else {
        // Slot is genuinely empty (genesis author's first write, or a brand-new
        // member's freshly-claimed slot) — a fresh write is safe.
        return Ok(MyChain {
            entries,
            overflow_keys,
        });
    };
    let Some(primary) = verify_payload(&primary_bytes, Some(me)) else {
        // Slot is OCCUPIED but its payload fails verification. Returning an empty
        // chain here would make `write_entry` rewrite the slot with only the new
        // entry, wiping our genesis/governance off the DHT. Refuse instead.
        return Err(GovernanceRuntimeError::PrimarySubkeyUnverifiable { slot: my_slot });
    };
    entries.extend(primary.entries);

    let mut next = primary.overflow_next;
    let mut visited: HashSet<String> = HashSet::new();
    while let Some(key) = next.take() {
        if overflow_keys.len() >= MAX_OVERFLOW_PAGES || !visited.insert(key.clone()) {
            break;
        }
        // A page that cannot be borrowed reads as empty.
        let page_bytes = read_subkey(deps, &key, OVERFLOW_SUBKEY, false)
            .await
            .ok()
            .flatten()
            .filter(|b| !b.is_empty());
        overflow_keys.push(key);
        let Some(page) = page_bytes
            .as_deref()
            .and_then(|b| verify_payload(b, Some(me)))
        else {
            break;
        };
        entries.extend(page.entries);
        next = page.overflow_next;
    }

    Ok(MyChain {
        entries,
        overflow_keys,
    })
}

/// Write one overflow page to an overflow record's subkey 0 as `writer` (the
/// derived owner keypair string), with M9.5 conflict detection + read-back
/// verify, mirroring the primary-subkey write in `apply::write_entry`.
pub async fn write_overflow_page<D: OverflowIo>(
    deps: &D,
    record_key: &str,
    page: &[GovernanceEntry],
    overflow_next: Option<String>,
    signing_key: &SigningKey,
    author: &PseudonymKey,
    writer: String,
) -> Result<(), GovernanceRuntimeError> {
    let (_payload, bytes) = build_signed_payload(page, overflow_next, signing_key, author)?;
    if bytes.len() > DFLT_SUBKEY_MAX_BYTES {
        return Err(GovernanceRuntimeError::SubkeyOverflow {
            bytes: bytes.len(),
            cap: DFLT_SUBKEY_MAX_BYTES,
        });
    }
    let lease = deps
        .acquire_record(record_key, Some(writer.clone()))
        .await?;
    let written = write_and_read_back(deps, lease, &bytes, writer).await;
    deps.release_record(lease).await;
    let verify = written?;
    if verify != bytes {
        return Err(GovernanceRuntimeError::VerifyMismatch {
            read: verify.len(),
            written: bytes.len(),
        });
    }
    Ok(())
}

/// Set the overflow subkey and read it back from the network.
async fn write_and_read_back<D: OverflowIo>(
    deps: &D,
    lease: LeaseId,
    bytes: &[u8],
    writer: String,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    if let Some(stale) = deps
        .set_dht_value(lease, OVERFLOW_SUBKEY, bytes.to_vec(), Some(writer))
        .await?
    {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }
    deps.get_dht_value(lease, OVERFLOW_SUBKEY, true)
        .await?
        .ok_or(GovernanceRuntimeError::VerifyEmpty)
}

/// Format the derived overflow-record owner keypair for `page_index` into the
/// adapter's writer-string form. Page 0 is the primary subkey; overflow pages
/// start at index 1.
pub fn overflow_owner_writer<D: OverflowIo>(
    deps: &D,
    identity_secret: &[u8; 32],
    community_id: &str,
    page_index: u32,
) -> String {
    let owner =
        derive::derive_governance_overflow_keypair(identity_secret, community_id, page_index);
    deps.format_writer_keypair(owner.verifying_key().to_bytes(), owner.to_bytes())
}

/// Outcome of [`read_governance_with_overflow`]: the `(author, entries)` pairs
/// ready for [`rekindle_governance::merge`], plus every overflow record key the
/// pass followed. Callers register `overflow_keys` into the community's record
/// inventory (Mutual Aid §14.1 — a reader keeps alive every record it reads) so
/// the followed pages share the keepalive / rehydration / teardown path.
#[derive(Debug, Default)]
pub struct GovernanceReadout {
    pub authored: Vec<(PseudonymKey, Vec<GovernanceEntry>)>,
    pub overflow_keys: Vec<String>,
}

/// Read a governance record's occupied subkeys, W26-verify each payload, and
/// follow every author's `overflow_next` chain. Overflow pages are emitted as
/// their own pairs — merge flattens and re-sorts by Lamport, so paging is
/// invisible to the merged state. Cycle-guarded by a shared visited-key set and
/// depth-capped at [`MAX_OVERFLOW_PAGES`] per chain. A momentarily-unreachable
/// page is reported (warn, not silent) so a dormant community's truncation is
/// diagnosable; durability (warming + rehydration of the returned
/// `overflow_keys`) is what actually keeps the page reachable.
pub async fn read_governance_with_overflow<D: OverflowIo>(
    deps: &D,
    gov_key: &str,
    occupied: &[u32],
) -> GovernanceReadout {
    let mut out: Vec<(PseudonymKey, Vec<GovernanceEntry>)> = Vec::new();
    let mut overflow_keys: Vec<String> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();

    for &subkey in occupied {
        if deps.stop_requested() {
            break;
        }
        let Ok(Some(bytes)) = read_subkey(deps, gov_key, subkey, false).await else {
            continue;
        };
        if bytes.is_empty() {
            continue;
        }
        let Some(payload) = verify_payload(&bytes, None) else {
            continue;
        };
        let author = payload.author_pseudonym.clone();
        out.push((author.clone(), payload.entries));

        let mut next = payload.overflow_next;
        let mut depth = 0usize;
        while let Some(key) = next.take() {
            if deps.stop_requested() || depth >= MAX_OVERFLOW_PAGES || !visited.insert(key.clone())
            {
                break;
            }
            depth += 1;
            // Register the key regardless of this read's success: Mutual Aid
            // keeps us warming + rehydrating every overflow record we follow, so
            // a momentarily-unreachable page must stay in the inventory for a
            // later warm cycle to re-fetch.
            overflow_keys.push(key.clone());
            let Ok(Some(page_bytes)) = read_subkey(deps, &key, OVERFLOW_SUBKEY, false).await else {
                tracing::warn!(
                    overflow_key = %key,
                    "overflow page unreachable — governance may be truncated until a holder warms it",
                );
                break;
            };
            if page_bytes.is_empty() {
                tracing::warn!(
                    overflow_key = %key,
                    "overflow page empty — governance may be truncated",
                );
                break;
            }
            let Some(page) = verify_payload(&page_bytes, Some(&author)) else {
                tracing::warn!(
                    overflow_key = %key,
                    "overflow page failed verification — dropping (possible chain redirect)",
                );
                break;
            };
            out.push((author.clone(), page.entries));
            next = page.overflow_next;
        }
    }

    GovernanceReadout {
        authored: out,
        overflow_keys,
    }
}

#[cfg(test)]
mod tests;

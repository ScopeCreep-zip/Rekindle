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
    async fn get_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    async fn set_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError>;
    async fn open_dht_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError>;
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String;
}

#[async_trait::async_trait]
impl<D: GovernanceRuntimeDeps> OverflowIo for D {
    async fn get_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        GovernanceRuntimeDeps::get_dht_value(self, record_key, subkey, force_refresh).await
    }
    async fn set_dht_value(
        &self,
        record_key: &str,
        subkey: u32,
        value: Vec<u8>,
        writer: Option<String>,
    ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
        GovernanceRuntimeDeps::set_dht_value(self, record_key, subkey, value, writer).await
    }
    async fn open_dht_record(
        &self,
        record_key: &str,
        writer: Option<String>,
    ) -> Result<(), GovernanceRuntimeError> {
        GovernanceRuntimeDeps::open_dht_record(self, record_key, writer).await
    }
    fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
        GovernanceRuntimeDeps::format_writer_keypair(self, ed_public, ed_secret)
    }
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

    let primary_bytes = deps
        .get_dht_value(gov_key, my_slot, false)
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
        // Read-only open (no writer); tolerate open failure as an empty page.
        let _ = deps.open_dht_record(&key, None).await;
        let page_bytes = deps
            .get_dht_value(&key, OVERFLOW_SUBKEY, false)
            .await?
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
    deps.open_dht_record(record_key, Some(writer.clone()))
        .await?;
    let outcome = deps
        .set_dht_value(record_key, OVERFLOW_SUBKEY, bytes.clone(), Some(writer))
        .await?;
    if let Some(stale) = outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }
    let verify = deps
        .get_dht_value(record_key, OVERFLOW_SUBKEY, true)
        .await?
        .ok_or(GovernanceRuntimeError::VerifyEmpty)?;
    if verify != bytes {
        return Err(GovernanceRuntimeError::VerifyMismatch {
            read: verify.len(),
            written: bytes.len(),
        });
    }
    Ok(())
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
        let Ok(Some(bytes)) = deps.get_dht_value(gov_key, subkey, false).await else {
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
            if depth >= MAX_OVERFLOW_PAGES || !visited.insert(key.clone()) {
                break;
            }
            depth += 1;
            // Register the key regardless of this read's success: Mutual Aid
            // keeps us warming + rehydrating every overflow record we follow, so
            // a momentarily-unreachable page must stay in the inventory for a
            // later warm cycle to re-fetch.
            overflow_keys.push(key.clone());
            let _ = deps.open_dht_record(&key, None).await;
            let Ok(Some(page_bytes)) = deps.get_dht_value(&key, OVERFLOW_SUBKEY, false).await
            else {
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
mod tests {
    use super::*;
    use rekindle_types::id::{ChannelId, RoleId};

    fn channel_entry(name: &str, lamport: u64) -> GovernanceEntry {
        GovernanceEntry::ChannelCreated {
            channel_id: ChannelId([0u8; 16]),
            name: name.to_string(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport,
        }
    }

    fn role_entry(lamport: u64) -> GovernanceEntry {
        GovernanceEntry::RoleAssignment {
            target: PseudonymKey([1u8; 32]),
            role_id: RoleId([0u8; 16]),
            lamport,
        }
    }

    fn entry_size(e: &GovernanceEntry) -> usize {
        serde_json::to_vec(e).unwrap().len()
    }

    #[test]
    fn single_page_when_under_budget() {
        let entries = vec![role_entry(1), role_entry(2), role_entry(3)];
        let pages = partition_into_pages(entries.clone(), 100_000, 100_000).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].len(), 3);
    }

    #[test]
    fn splits_into_two_pages_at_budget() {
        // Budget that fits ~2 role entries on the primary page, forcing overflow.
        let one = entry_size(&role_entry(1));
        let primary_budget = one * 2 + one / 2; // room for exactly 2
        let entries = vec![role_entry(1), role_entry(2), role_entry(3), role_entry(4)];
        let pages = partition_into_pages(entries, primary_budget, primary_budget).unwrap();
        assert!(
            pages.len() >= 2,
            "expected a spill, got {} page(s)",
            pages.len()
        );
        assert_eq!(
            pages[0].len(),
            2,
            "primary page should hold exactly 2 entries"
        );
    }

    #[test]
    fn oversized_primary_entry_spills_to_overflow_page() {
        // An entry that exceeds the tiny primary budget but fits the overflow
        // budget must land on an overflow page, leaving the primary empty.
        let big = channel_entry(&"x".repeat(400), 1);
        let sz = entry_size(&big);
        let pages = partition_into_pages(vec![big], sz / 2, sz * 2).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(
            pages[0].is_empty(),
            "primary page should have spilled empty"
        );
        assert_eq!(pages[1].len(), 1);
    }

    #[test]
    fn unsplittable_entry_errors() {
        let big = channel_entry(&"y".repeat(500), 1);
        let sz = entry_size(&big);
        let err = partition_into_pages(vec![big], sz / 2, sz / 2);
        assert!(matches!(err, Err(n) if n == sz));
    }

    #[test]
    fn roundtrip_partition_then_concat_preserves_entries() {
        // Paging must be invisible to merge: flattening the pages back must
        // reproduce the input exactly (same entries, same order).
        let input: Vec<GovernanceEntry> = (0..40)
            .map(|i| {
                if i % 2 == 0 {
                    channel_entry(&format!("chan-{i}"), i)
                } else {
                    role_entry(i)
                }
            })
            .collect();
        let one = entry_size(&role_entry(1));
        let pages = partition_into_pages(input.clone(), one * 3, one * 7).unwrap();
        assert!(pages.len() >= 2, "test budgets should force multiple pages");
        let flattened: Vec<GovernanceEntry> = pages.into_iter().flatten().collect();
        assert_eq!(flattened, input);
    }

    /// In-memory DHT keyed by `(record_key, subkey)`. Implements only the
    /// four-method [`OverflowIo`] surface (the blanket impl covers production;
    /// tests need just this), so the write/read/follow cycle runs with no Veilid.
    #[derive(Default)]
    struct MockDht {
        store: parking_lot::Mutex<std::collections::HashMap<(String, u32), Vec<u8>>>,
    }

    #[async_trait::async_trait]
    impl OverflowIo for MockDht {
        async fn get_dht_value(
            &self,
            record_key: &str,
            subkey: u32,
            _force_refresh: bool,
        ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
            Ok(self
                .store
                .lock()
                .get(&(record_key.to_string(), subkey))
                .cloned())
        }
        async fn set_dht_value(
            &self,
            record_key: &str,
            subkey: u32,
            value: Vec<u8>,
            _writer: Option<String>,
        ) -> Result<Option<Vec<u8>>, GovernanceRuntimeError> {
            self.store
                .lock()
                .insert((record_key.to_string(), subkey), value);
            Ok(None)
        }
        async fn open_dht_record(
            &self,
            _record_key: &str,
            _writer: Option<String>,
        ) -> Result<(), GovernanceRuntimeError> {
            Ok(())
        }
        fn format_writer_keypair(&self, ed_public: [u8; 32], ed_secret: [u8; 32]) -> String {
            format!("{}:{}", hex::encode(ed_public), hex::encode(ed_secret))
        }
    }

    /// End-to-end: one author writes 30 channels that overflow the primary
    /// subkey, the primary carries an `overflow_next` pointer, and a fresh read
    /// reassembles all 30 across the chain — paging is invisible to the reader.
    #[tokio::test]
    async fn write_then_read_reassembles_across_overflow_pages() {
        let mock = MockDht::default();
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let author = PseudonymKey(signing.verifying_key().to_bytes());
        let gov_key = "govkey";
        let my_slot = 5u32;

        let input: Vec<GovernanceEntry> = (0..30u64)
            .map(|i| channel_entry(&format!("chan{i:02}"), i))
            .collect();
        let one = entry_size(&input[0]);
        // Budgets force the primary to hold ~4 entries and spill the rest into
        // overflow pages of ~9 — several pages, exercising the full chain.
        let pages = partition_into_pages(input.clone(), one * 4, one * 9).unwrap();
        assert!(pages.len() >= 2, "30 channels should span multiple pages");

        // Write overflow pages highest-index-first so each parent points at an
        // already-written child, then the primary subkey.
        let mut next_key: Option<String> = None;
        for i in (1..pages.len()).rev() {
            let rec_key = format!("ovl-{i}");
            write_overflow_page(
                &mock,
                &rec_key,
                &pages[i],
                next_key.clone(),
                &signing,
                &author,
                "writer".to_string(),
            )
            .await
            .unwrap();
            next_key = Some(rec_key);
        }
        let (primary, bytes) =
            build_signed_payload(&pages[0], next_key, &signing, &author).unwrap();
        assert!(
            primary.overflow_next.is_some(),
            "primary subkey must point at the first overflow record"
        );
        mock.set_dht_value(gov_key, my_slot, bytes, Some("slot".into()))
            .await
            .unwrap();

        // read_my_chain (write path) reassembles the full log + every chain key.
        let chain = read_my_chain(&mock, gov_key, my_slot, &author)
            .await
            .unwrap();
        assert_eq!(
            chain.entries, input,
            "write-path read must reassemble in order"
        );
        assert_eq!(chain.overflow_keys.len(), pages.len() - 1);

        // read_governance_with_overflow (read path) → flatten == every entry,
        // and it reports every overflow key it followed (for inventory register).
        let readout = read_governance_with_overflow(&mock, gov_key, &[my_slot]).await;
        let flattened: Vec<GovernanceEntry> =
            readout.authored.into_iter().flat_map(|(_, e)| e).collect();
        assert_eq!(flattened, input, "read path must reproduce all 30 channels");
        assert_eq!(
            readout.overflow_keys.len(),
            pages.len() - 1,
            "read path must report every followed overflow key",
        );
    }

    /// An overflow page signed by a *different* author than the primary that
    /// pointed at it is rejected — a rogue member can't redirect the chain.
    #[tokio::test]
    async fn overflow_page_with_wrong_author_is_dropped() {
        let mock = MockDht::default();
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let author = PseudonymKey(signing.verifying_key().to_bytes());
        let attacker = SigningKey::from_bytes(&[9u8; 32]);
        let attacker_pseudo = PseudonymKey(attacker.verifying_key().to_bytes());

        // Primary (page 0) authored by `author`, pointing at an overflow record
        // that an attacker signs with their own pseudonym.
        write_overflow_page(
            &mock,
            "ovl-1",
            &[channel_entry("evil", 2)],
            None,
            &attacker,
            &attacker_pseudo,
            "writer".to_string(),
        )
        .await
        .unwrap();
        let (_p, bytes) = build_signed_payload(
            &[channel_entry("good", 1)],
            Some("ovl-1".to_string()),
            &signing,
            &author,
        )
        .unwrap();
        mock.set_dht_value("govkey", 0, bytes, None).await.unwrap();

        let readout = read_governance_with_overflow(&mock, "govkey", &[0]).await;
        let flattened: Vec<GovernanceEntry> =
            readout.authored.into_iter().flat_map(|(_, e)| e).collect();
        assert_eq!(
            flattened.len(),
            1,
            "mis-authored overflow page must be dropped"
        );
    }

    /// A genuinely empty primary slot is safe to write fresh → empty chain.
    #[tokio::test]
    async fn read_my_chain_empty_slot_returns_empty_chain() {
        let mock = MockDht::default();
        let author = PseudonymKey([4u8; 32]);
        let chain = read_my_chain(&mock, "govkey", 5, &author).await.unwrap();
        assert!(chain.entries.is_empty());
        assert!(chain.overflow_keys.is_empty());
    }

    /// An occupied-but-unverifiable primary slot must NOT return an empty chain
    /// (which would let `write_entry` overwrite and wipe genesis) — it errors.
    #[tokio::test]
    async fn read_my_chain_occupied_unverifiable_errors() {
        let mock = MockDht::default();
        let author = PseudonymKey([4u8; 32]);
        // Garbage that won't deserialize into a verifiable payload.
        mock.set_dht_value("govkey", 5, b"not-a-payload".to_vec(), None)
            .await
            .unwrap();
        let err = read_my_chain(&mock, "govkey", 5, &author).await;
        assert!(
            matches!(
                err,
                Err(GovernanceRuntimeError::PrimarySubkeyUnverifiable { slot: 5 })
            ),
            "occupied garbage must refuse to overwrite, got {err:?}"
        );
    }
}

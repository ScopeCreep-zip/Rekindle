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
    let (primary, bytes) = build_signed_payload(&pages[0], next_key, &signing, &author).unwrap();
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

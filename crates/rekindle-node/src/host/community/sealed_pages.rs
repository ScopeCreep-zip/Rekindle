//! Record warming of sealed pages (plan E3.2 S7, ADR 0011 "Sealed pages",
//! architecture §14.1 mutual aid). A sealed DFLT(1) page has no owner key
//! once written, so it is never written again; Veilid 0.5.8 keeps no record
//! by time, and a remote store is an LRU of `remote_max_records` (128,
//! `veilid_config.rs:356`, `record_store/mod.rs:2-5`), so a page nobody uses
//! is evicted. Any node holding a local copy keeps it alive: a `Hold`
//! re-open rehydrates it and re-pushes the stored signed value
//! (`open_record.rs:62-81`, `rehydrate.rs:309-330`).
//!
//! The warm set is bounded by the latest endorsed checkpoint: its
//! directory pages (ceil(dir_size / 128)), plus our own op pages holding
//! ops beyond its cut (older ones are folded into the checkpoint a joiner
//! reads). A newer endorsed checkpoint replaces the directory and prunes
//! the own pages it folds; leaving drops the community, and the set with it.

use std::collections::BTreeMap;

use rekindle_codec::capnp_codec::chain::PagePointer;
use rekindle_protocol::dht::pool::keepalive::WarmRecord;
use rekindle_types::governance::OpRef;
use rekindle_types::id::PseudonymKey;

use super::state::CommunityState;

/// The sealed pages one community keeps warm. Nothing is persisted: own
/// pages are learned from our appends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SealedPages {
    /// The checkpoint height whose directory pages are held.
    height: u64,
    directory: Vec<String>,
    /// Our own op pages beyond that checkpoint's cut: first seq → (last
    /// seq, page key).
    own_ops: BTreeMap<u64, (u64, String)>,
}

impl SealedPages {
    /// The checkpoint at `height` with directory `pages` and cut heads
    /// `cut`: held unless a higher one already is (a proposal is held from
    /// its sealing until a newer height is endorsed), and our own pages
    /// whose every op is at or below our head in the cut are released.
    pub fn checkpoint(
        &mut self,
        height: u64,
        pages: &[String],
        cut: &[OpRef],
        me: Option<&PseudonymKey>,
    ) {
        if height < self.height {
            return;
        }
        self.height = height;
        self.directory = pages.to_vec();
        let folded = (cut.iter())
            .filter(|head| Some(&head.author) == me)
            .map(|head| head.author_seq)
            .max();
        if let Some(folded) = folded {
            self.own_ops.retain(|_, (last, _)| *last > folded);
        }
    }

    /// One of our own op pages, just sealed by an append.
    pub fn own_page(&mut self, page: &PagePointer) {
        self.own_ops
            .insert(page.first_seq, (page.last_seq, page.page_key.clone()));
    }

    /// Every page held, each sealed, in key order.
    #[must_use]
    pub fn records(&self) -> Vec<WarmRecord> {
        let mut keys: Vec<String> = (self.directory.iter().cloned())
            .chain(self.own_ops.values().map(|(_, key)| key.clone()))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter().map(WarmRecord::sealed).collect()
    }
}

/// The sealed pages `community` keeps warm.
#[must_use]
pub fn warm_sealed_pages(community: &CommunityState) -> Vec<WarmRecord> {
    community.sealed_pages.records()
}

#[cfg(test)]
#[path = "sealed_pages_tests.rs"]
mod tests;

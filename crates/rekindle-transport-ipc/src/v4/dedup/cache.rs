//! Sender-side and receiver-side ContentHash caches with LRU eviction.

use std::collections::{HashMap, HashSet, VecDeque};
use crate::v4::wire::clearance::Clearance;

// ── Sender Cache ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SenderCacheConfig {
    pub max_entries: usize,
    pub max_bytes: u64,
}

impl Default for SenderCacheConfig {
    fn default() -> Self {
        Self { max_entries: 1024, max_bytes: 1_073_741_824 }
    }
}

pub struct SenderCacheEntry {
    pub payload_size_bytes: u64,
    pub chunk_count: u32,
    pub emission_count: u32,
    acked_by: HashSet<(uuid::Uuid, [u8; 32])>, // (session_id, peer_id)
}

pub struct SenderCache {
    config: SenderCacheConfig,
    entries: HashMap<[u8; 32], SenderCacheEntry>,
    lru_order: VecDeque<[u8; 32]>,
    total_bytes: u64,
}

impl SenderCache {
    pub fn new(config: SenderCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            lru_order: VecDeque::new(),
            total_bytes: 0,
        }
    }

    pub fn store(&mut self, content_hash: [u8; 32], payload_size_bytes: u64, chunk_count: u32) {
        if self.config.max_entries == 0 && self.config.max_bytes == 0 {
            return;
        }
        if let Some(existing) = self.entries.get_mut(&content_hash) {
            existing.emission_count += 1;
            self.touch_lru(&content_hash);
            return;
        }
        self.entries.insert(content_hash, SenderCacheEntry {
            payload_size_bytes,
            chunk_count,
            emission_count: 0,
            acked_by: HashSet::new(),
        });
        self.lru_order.push_back(content_hash);
        self.total_bytes += payload_size_bytes;
        self.evict();
    }

    pub fn lookup(&mut self, content_hash: &[u8; 32]) -> Option<&SenderCacheEntry> {
        if self.entries.contains_key(content_hash) {
            self.touch_lru(content_hash);
        }
        self.entries.get(content_hash)
    }

    pub fn mark_acked(&mut self, content_hash: &[u8; 32], session_id: uuid::Uuid, peer_id: [u8; 32]) {
        if let Some(entry) = self.entries.get_mut(content_hash) {
            entry.acked_by.insert((session_id, peer_id));
        }
    }

    pub fn is_acked(&self, content_hash: &[u8; 32], session_id: uuid::Uuid, peer_id: [u8; 32]) -> bool {
        self.entries.get(content_hash)
            .is_some_and(|e| e.acked_by.contains(&(session_id, peer_id)))
    }

    pub fn record_emission(&mut self, content_hash: &[u8; 32]) {
        if let Some(entry) = self.entries.get_mut(content_hash) {
            entry.emission_count += 1;
            self.touch_lru(content_hash);
        }
    }

    pub fn remove(&mut self, content_hash: &[u8; 32]) -> bool {
        if let Some(removed) = self.entries.remove(content_hash) {
            self.total_bytes = self.total_bytes.saturating_sub(removed.payload_size_bytes);
            self.lru_order.retain(|h| h != content_hash);
            true
        } else {
            false
        }
    }

    pub fn close_session(&mut self, session_id: uuid::Uuid) {
        for entry in self.entries.values_mut() {
            entry.acked_by.retain(|(sid, _)| *sid != session_id);
        }
    }

    fn touch_lru(&mut self, hash: &[u8; 32]) {
        if let Some(pos) = self.lru_order.iter().position(|h| h == hash) {
            self.lru_order.remove(pos);
            self.lru_order.push_back(*hash);
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > self.config.max_entries && !self.lru_order.is_empty() {
            if let Some(oldest) = self.lru_order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.payload_size_bytes);
                }
            }
        }
        while self.total_bytes > self.config.max_bytes && !self.lru_order.is_empty() {
            if let Some(oldest) = self.lru_order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.payload_size_bytes);
                }
            }
        }
    }
}

// ── Receiver Cache ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ReceiverCacheConfig {
    pub max_entries: usize,
    pub max_bytes: u64,
}

impl Default for ReceiverCacheConfig {
    fn default() -> Self {
        Self { max_entries: 1024, max_bytes: 1_073_741_824 }
    }
}

pub struct ReceiverCacheEntry {
    pub payload: Vec<u8>,
    pub payload_size_bytes: u64,
    pub chunk_count: u32,
    pub clearance: Clearance,
    pub receipt_count: u32,
}

pub struct ReceiverCache {
    config: ReceiverCacheConfig,
    entries: HashMap<[u8; 32], ReceiverCacheEntry>,
    lru_order: VecDeque<[u8; 32]>,
    total_bytes: u64,
}

impl ReceiverCache {
    pub fn new(config: ReceiverCacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            lru_order: VecDeque::new(),
            total_bytes: 0,
        }
    }

    pub fn store(
        &mut self,
        content_hash: [u8; 32],
        payload: Vec<u8>,
        payload_size_bytes: u64,
        chunk_count: u32,
        clearance: Clearance,
    ) {
        if let Some(existing) = self.entries.get_mut(&content_hash) {
            existing.receipt_count += 1;
            existing.payload = payload;
            existing.payload_size_bytes = payload_size_bytes;
            existing.chunk_count = chunk_count;
            if clearance > existing.clearance {
                existing.clearance = clearance;
            }
            self.touch_lru(&content_hash);
            return;
        }
        let stored_bytes = payload.len() as u64;
        self.entries.insert(content_hash, ReceiverCacheEntry {
            payload,
            payload_size_bytes,
            chunk_count,
            clearance,
            receipt_count: 1,
        });
        self.lru_order.push_back(content_hash);
        self.total_bytes += stored_bytes;
        self.evict();
    }

    pub fn lookup(&self, content_hash: &[u8; 32]) -> Option<&ReceiverCacheEntry> {
        self.entries.get(content_hash)
    }

    pub fn remove(&mut self, content_hash: &[u8; 32]) -> bool {
        if let Some(removed) = self.entries.remove(content_hash) {
            self.total_bytes = self.total_bytes.saturating_sub(removed.payload.len() as u64);
            self.lru_order.retain(|h| h != content_hash);
            true
        } else {
            false
        }
    }

    fn touch_lru(&mut self, hash: &[u8; 32]) {
        if let Some(pos) = self.lru_order.iter().position(|h| h == hash) {
            self.lru_order.remove(pos);
            self.lru_order.push_back(*hash);
        }
    }

    fn evict(&mut self) {
        while self.entries.len() > self.config.max_entries && !self.lru_order.is_empty() {
            if let Some(oldest) = self.lru_order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.payload.len() as u64);
                }
            }
        }
        while self.total_bytes > self.config.max_bytes && !self.lru_order.is_empty() {
            if let Some(oldest) = self.lru_order.pop_front() {
                if let Some(removed) = self.entries.remove(&oldest) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.payload.len() as u64);
                }
            }
        }
    }
}

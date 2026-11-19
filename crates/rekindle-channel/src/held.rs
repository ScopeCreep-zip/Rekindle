//! The changes that arrive before their target (plan E3.3 rule E5): an
//! edit or delete whose message we do not hold yet waits here, keyed by
//! (author, op id), and is re-validated when the message lands.
//!
//! The cache is bounded as Signal bounds its early-arrival cache: at most
//! [`MAX_HELD_KEYS`] keys and [`MAX_HELD_BYTES`] bytes. A change past
//! either bound evicts the oldest held key first, so an author flooding
//! changes for posts that never come costs a reader a fixed amount.

use std::collections::{BTreeMap, VecDeque};

/// At most this many (author, op id) keys wait (E5, Signal's 100).
pub const MAX_HELD_KEYS: usize = 100;

/// At most this many bytes of held changes wait (E5, Signal's 5 MiB).
pub const MAX_HELD_BYTES: usize = 5 * 1024 * 1024;

/// One held change's key: the author and the message it names.
pub type HeldKey = (String, String);

/// The bounded cache of changes waiting for their target.
#[derive(Debug)]
pub struct HeldChanges<T> {
    held: BTreeMap<HeldKey, Vec<(T, usize)>>,
    /// Keys oldest first, for eviction.
    order: VecDeque<HeldKey>,
    bytes: usize,
    max_keys: usize,
    max_bytes: usize,
}

impl<T> Default for HeldChanges<T> {
    fn default() -> Self {
        Self::with_bounds(MAX_HELD_KEYS, MAX_HELD_BYTES)
    }
}

impl<T> HeldChanges<T> {
    /// A cache holding at most `max_keys` keys and `max_bytes` bytes.
    #[must_use]
    pub fn with_bounds(max_keys: usize, max_bytes: usize) -> Self {
        Self {
            held: BTreeMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_keys,
            max_bytes,
        }
    }

    /// Hold `change` (of `size` bytes) by `author` for `message_id`,
    /// evicting the oldest keys while a bound is passed. A change larger
    /// than the byte bound on its own is not held.
    pub fn hold(&mut self, author: &str, message_id: &str, change: T, size: usize) {
        if size > self.max_bytes {
            return;
        }
        let key = (author.to_ascii_lowercase(), message_id.to_owned());
        if !self.held.contains_key(&key) {
            self.order.push_back(key.clone());
        }
        self.held.entry(key).or_default().push((change, size));
        self.bytes += size;
        while self.held.len() > self.max_keys || self.bytes > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(gone) = self.held.remove(&oldest) {
                self.bytes -= gone.iter().map(|(_, size)| size).sum::<usize>();
            }
        }
    }

    /// Every change `author` made to `message_id` that waits, oldest
    /// first; they leave the cache.
    pub fn take(&mut self, author: &str, message_id: &str) -> Vec<T> {
        let key = (author.to_ascii_lowercase(), message_id.to_owned());
        let Some(changes) = self.held.remove(&key) else {
            return Vec::new();
        };
        self.order.retain(|k| *k != key);
        self.bytes -= changes.iter().map(|(_, size)| size).sum::<usize>();
        changes.into_iter().map(|(change, _)| change).collect()
    }

    /// How many keys wait.
    #[must_use]
    pub fn len(&self) -> usize {
        self.held.len()
    }

    /// Nothing waits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_change_is_taken_once_by_its_author_and_message() {
        let mut held = HeldChanges::default();
        held.hold("AA", "m1", 1, 10);
        held.hold("aa", "m1", 2, 10);
        assert!(held.take("bb", "m1").is_empty(), "another author's");
        assert_eq!(held.take("aa", "m1"), vec![1, 2]);
        assert!(held.take("aa", "m1").is_empty());
        assert!(held.is_empty());
    }

    #[test]
    fn the_key_bound_evicts_the_oldest_key() {
        let mut held = HeldChanges::default();
        for i in 0..=MAX_HELD_KEYS {
            held.hold("aa", &format!("m{i}"), i, 1);
        }
        assert_eq!(held.len(), MAX_HELD_KEYS);
        assert!(held.take("aa", "m0").is_empty(), "the oldest key went");
        assert_eq!(
            held.take("aa", &format!("m{MAX_HELD_KEYS}")),
            vec![MAX_HELD_KEYS]
        );
    }

    #[test]
    fn the_byte_bound_evicts_the_oldest_key() {
        let mut held = HeldChanges::with_bounds(10, 100);
        held.hold("aa", "m1", 1, 60);
        held.hold("aa", "m2", 2, 60);
        assert!(
            held.take("aa", "m1").is_empty(),
            "past 100 bytes the oldest went"
        );
        assert_eq!(held.take("aa", "m2"), vec![2]);
        held.hold("aa", "m3", 3, 101);
        assert!(held.is_empty(), "a change past the bound alone is not held");
    }
}

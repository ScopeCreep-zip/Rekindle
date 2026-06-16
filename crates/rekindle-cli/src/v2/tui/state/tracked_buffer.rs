//! Generation-tracked mutable buffer — VecDeque with automatic cache invalidation.
//!
//! Every mutating method increments `generation`. Render caches compare their
//! stored generation against the buffer's to decide whether to rebuild ListItems.
//! The invariant: if the data changed, generation changed. No exception.
//!
//! This type is the single source of truth for message collections in both
//! DM threads and channel views. Future conversation types (group DMs, thread
//! message panels) compose this buffer instead of reimplementing generation tracking.

use std::collections::VecDeque;

/// A generation-tracked VecDeque. Every mutation increments `generation`.
/// Read access does not change generation.
///
/// `generation` starts at 0. Render caches start at `u64::MAX`.
/// The first `needs_rebuild` check always returns true.
#[derive(Debug)]
pub struct TrackedBuffer<T> {
    inner: VecDeque<T>,
    generation: u64,
}

impl<T> TrackedBuffer<T> {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
            generation: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
            generation: 0,
        }
    }

    // ── Read access (no generation change) ─────────────────────

    /// Current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Borrow the inner VecDeque for read-only operations that need
    /// the concrete type (e.g., passing to render functions, iteration,
    /// random access, length checks).
    pub fn as_deque(&self) -> &VecDeque<T> {
        &self.inner
    }

    // ── Mutating access (generation auto-increments) ───────────

    /// Append an item, evicting the oldest if at or above `cap`.
    /// Returns the evicted item if eviction occurred.
    /// Generation increments regardless of eviction.
    pub fn push_capped(&mut self, item: T, cap: usize) -> Option<T> {
        let evicted = if self.inner.len() >= cap {
            self.inner.pop_front()
        } else {
            None
        };
        self.inner.push_back(item);
        self.generation += 1;
        evicted
    }

    /// Retain only items matching the predicate. Generation increments.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, f: F) {
        self.inner.retain(f);
        self.generation += 1;
    }

    /// Replace contents with `new`, preserving items from the old buffer
    /// that match `keep`. Preserved items are appended after `new`.
    ///
    /// This is the race-safe history replacement: `new` is the daemon's
    /// historical data, `keep` preserves optimistic sends and real-time
    /// messages that arrived after the history request was sent.
    pub fn replace_preserving<I, F>(&mut self, new: I, mut keep: F)
    where
        I: IntoIterator<Item = T>,
        F: FnMut(&T) -> bool,
    {
        let preserved: Vec<T> = self.inner.drain(..).filter(|item| keep(item)).collect();
        self.inner = new.into_iter().collect();
        for item in preserved {
            self.inner.push_back(item);
        }
        self.generation += 1;
    }

    /// Search backwards for an item matching `predicate`, apply `mutate` to it.
    /// Returns true if a match was found. Generation increments only if found.
    pub fn find_mut_rev<P, M>(&mut self, mut predicate: P, mutate: M) -> bool
    where
        P: FnMut(&T) -> bool,
        M: FnOnce(&mut T),
    {
        if let Some(item) = self.inner.iter_mut().rev().find(|item| predicate(item)) {
            mutate(item);
            self.generation += 1;
            true
        } else {
            false
        }
    }
}

impl<T> Default for TrackedBuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_starts_at_zero() {
        let buf: TrackedBuffer<u32> = TrackedBuffer::new();
        assert_eq!(buf.generation(), 0);
    }

    #[test]
    fn push_capped_increments_generation() {
        let mut buf = TrackedBuffer::new();
        let _ = buf.push_capped(1, 10);
        assert_eq!(buf.generation(), 1);
        let _ = buf.push_capped(2, 10);
        assert_eq!(buf.generation(), 2);
    }

    #[test]
    fn push_capped_evicts_oldest() {
        let mut buf = TrackedBuffer::new();
        let _ = buf.push_capped(1, 3);
        let _ = buf.push_capped(2, 3);
        let _ = buf.push_capped(3, 3);
        let evicted = buf.push_capped(4, 3);
        assert_eq!(evicted, Some(1));
        assert_eq!(buf.as_deque().len(), 3);
        assert_eq!(buf.as_deque()[0], 2);
        assert_eq!(buf.generation(), 4);
    }

    #[test]
    fn replace_preserving_keeps_matching() {
        let mut buf = TrackedBuffer::new();
        let _ = buf.push_capped(1, 100);
        let _ = buf.push_capped(2, 100);
        let _ = buf.push_capped(3, 100);
        buf.replace_preserving(vec![10, 20], |&item| item > 2);
        assert_eq!(buf.as_deque().len(), 3);
        assert_eq!(buf.as_deque()[0], 10);
        assert_eq!(buf.as_deque()[1], 20);
        assert_eq!(buf.as_deque()[2], 3);
    }

    #[test]
    fn find_mut_rev_only_increments_on_match() {
        let mut buf = TrackedBuffer::new();
        let _ = buf.push_capped(1, 100);
        let _ = buf.push_capped(2, 100);
        let gen_before = buf.generation();
        let found = buf.find_mut_rev(|&item| item == 99, |item| *item = 0);
        assert!(!found);
        assert_eq!(buf.generation(), gen_before);

        let found = buf.find_mut_rev(|&item| item == 2, |item| *item = 200);
        assert!(found);
        assert_eq!(buf.generation(), gen_before + 1);
        assert_eq!(buf.as_deque()[1], 200);
    }

    #[test]
    fn retain_increments_generation() {
        let mut buf = TrackedBuffer::new();
        let _ = buf.push_capped(1, 100);
        let _ = buf.push_capped(2, 100);
        let _ = buf.push_capped(3, 100);
        let gen_before = buf.generation();
        buf.retain(|&item| item != 2);
        assert_eq!(buf.as_deque().len(), 2);
        assert_eq!(buf.generation(), gen_before + 1);
    }

    #[test]
    fn with_capacity_preallocates() {
        let buf: TrackedBuffer<u32> = TrackedBuffer::with_capacity(5000);
        assert_eq!(buf.generation(), 0);
        assert!(buf.as_deque().capacity() >= 5000);
    }
}

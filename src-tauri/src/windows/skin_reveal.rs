//! A reveal needs the same stylesheet in the native allocation and in the
//! committed document. The deadline includes the time spent waiting for either.

use std::time::{Duration, Instant};

pub(super) const LOAD_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Show {
    First,
    Raise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkinRevealEffect {
    Wait,
    Skinned(Show),
    RevealAndReportTimeout(Show),
    Failed(Show),
}

#[derive(Default)]
pub(super) struct SkinRevealGate {
    native: Option<(String, Instant)>,
    committed: Option<(String, Instant)>,
    pending: Option<(Instant, Show)>,
    failed: bool,
}

impl SkinRevealGate {
    /// Returns a deadline only for the first request in a pending reveal.
    pub(super) fn request(&mut self, now: Instant, show: Show) -> Option<Instant> {
        if let Some((_, pending)) = &mut self.pending {
            if show == Show::Raise {
                *pending = show;
            }
            return None;
        }
        let end = now + LOAD_LIMIT;
        self.pending = Some((end, show));
        Some(end)
    }

    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn failed(&self) -> bool {
        self.failed
    }

    fn ready_before(&self, deadline: Instant) -> bool {
        self.native
            .as_ref()
            .zip(self.committed.as_ref())
            .is_some_and(|((native, allocated), (committed, rendered))| {
                native == committed && *allocated < deadline && *rendered < deadline
            })
    }

    pub(super) fn allocate(&mut self, href: Option<&str>, now: Instant) {
        if self.native.as_ref().map(|(href, _)| href.as_str()) != href {
            self.native = href.map(|href| (href.to_owned(), now));
        }
    }

    pub(super) fn commit(&mut self, href: String, now: Instant) {
        if self
            .committed
            .as_ref()
            .is_none_or(|(shown, _)| *shown != href)
        {
            self.committed = Some((href, now));
        }
    }

    pub(super) fn new_document(&mut self) {
        self.committed = None;
        self.failed = false;
    }

    pub(super) fn poll(&mut self, now: Instant) -> SkinRevealEffect {
        let Some((end, show)) = self.pending else {
            return SkinRevealEffect::Wait;
        };
        let effect = if self.failed {
            SkinRevealEffect::Failed(show)
        } else if self.ready_before(end) {
            SkinRevealEffect::Skinned(show)
        } else if now >= end {
            self.failed = true;
            SkinRevealEffect::RevealAndReportTimeout(show)
        } else {
            return SkinRevealEffect::Wait;
        };
        self.pending = None;
        effect
    }
}

#[cfg(test)]
#[path = "skin_reveal_tests.rs"]
mod tests;

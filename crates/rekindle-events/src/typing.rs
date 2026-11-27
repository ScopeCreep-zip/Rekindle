//! Typing indicators decided in the backend (plan E2.3): the receiver's
//! expiry and the sender's refresh and idle stop.
//!
//! A typing start carries no lifetime of its own on the 1:1 wire, so the
//! receiver gives it one: a start that is not refreshed within
//! [`TYPING_TTL_MS`] lapses and the backend emits the stop, as Matrix's
//! `m.typing` `timeout` does. The sender refreshes a start every
//! [`TYPING_REFRESH_MS`] while input continues (shorter than the TTL, so a
//! steady typist never lapses) and stops after [`TYPING_IDLE_MS`] without
//! input. A window only reports its input; neither side keeps a timer in
//! the frontend.
//!
//! Both halves are pure state over explicit times (the Host's clock, C9.2)
//! keyed by [`TypingContext`], so the 1:1 path (E2.3), group typing (E2.6)
//! and channel typing (E3.4, with its own per-start TTL) share them; each
//! host drives the deadlines with its own timer.

use std::collections::{HashMap, HashSet};

use rekindle_types::subscription_events::TypingContext;

/// How long a typing start lasts at the receiver without a refresh.
pub const TYPING_TTL_MS: u64 = 5_000;
/// How often a sender repeats its start while input continues.
pub const TYPING_REFRESH_MS: u64 = 3_000;
/// How long a sender waits after the last input before it stops.
pub const TYPING_IDLE_MS: u64 = 3_000;

// A steady typist's refresh, and an idle sender's stop, both come before
// the receiver would lapse the start.
const _: () = assert!(TYPING_REFRESH_MS < TYPING_TTL_MS && TYPING_IDLE_MS < TYPING_TTL_MS);

/// Where a deadline stands when its timer fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lapse {
    /// The deadline passed: the entry is gone and its stop is due.
    Ended,
    /// The entry was refreshed: look again at this time (ms).
    Pending(u64),
    /// Nothing is held (stopped, or already ended).
    Gone,
}

/// The receiver's side: who is typing where, and when each start lapses.
#[derive(Debug, Default)]
pub struct TypingExpiry {
    deadlines: HashMap<(TypingContext, String), u64>,
}

impl TypingExpiry {
    /// `who` started (or is still) typing in `context` at `now_ms`; the
    /// start lapses at `now_ms + ttl_ms`. True when `who` was not already
    /// typing there: the start is news and its timer is due.
    pub fn start(&mut self, context: TypingContext, who: String, now_ms: u64, ttl_ms: u64) -> bool {
        self.deadlines
            .insert((context, who), now_ms.saturating_add(ttl_ms))
            .is_none()
    }

    /// `who` stopped typing in `context`. True when they were typing.
    pub fn stop(&mut self, context: &TypingContext, who: &str) -> bool {
        self.deadlines
            .remove(&(context.clone(), who.to_owned()))
            .is_some()
    }

    /// Where `who`'s start in `context` stands at `now_ms`; an ended one is
    /// removed.
    pub fn lapse(&mut self, context: &TypingContext, who: &str, now_ms: u64) -> Lapse {
        let key = (context.clone(), who.to_owned());
        match self.deadlines.get(&key).copied() {
            None => Lapse::Gone,
            Some(deadline) if deadline > now_ms => Lapse::Pending(deadline),
            Some(_) => {
                self.deadlines.remove(&key);
                Lapse::Ended
            }
        }
    }

    /// Forget everything (lock).
    pub fn clear(&mut self) {
        self.deadlines.clear();
    }
}

/// What a sender does with one input report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// A new typing session: send the start and arm the idle timer.
    Started,
    /// The session's start is due again: send it.
    Refresh,
    /// The last start is recent enough: send nothing.
    Quiet,
}

#[derive(Debug, Clone, Copy)]
struct Session {
    last_input_ms: u64,
    last_sent_ms: u64,
}

/// The sender's side: our typing sessions, one per context.
#[derive(Debug, Default)]
pub struct TypingSessions {
    sessions: HashMap<TypingContext, Session>,
    /// Drafts that hold text but whose start could not be sent (the
    /// channel's key had not arrived): no session, so the next input
    /// starts one, and the host sends their start when the key lands.
    waiting: HashSet<TypingContext>,
}

impl TypingSessions {
    /// The user typed in `context` at `now_ms`.
    pub fn input(&mut self, context: TypingContext, now_ms: u64) -> Input {
        match self.sessions.get_mut(&context) {
            None => {
                self.waiting.remove(&context);
                self.sessions.insert(
                    context,
                    Session {
                        last_input_ms: now_ms,
                        last_sent_ms: now_ms,
                    },
                );
                Input::Started
            }
            Some(session) => {
                session.last_input_ms = now_ms;
                if now_ms.saturating_sub(session.last_sent_ms) >= TYPING_REFRESH_MS {
                    session.last_sent_ms = now_ms;
                    Input::Refresh
                } else {
                    Input::Quiet
                }
            }
        }
    }

    /// End the session in `context` now (the draft was cleared, or the
    /// message was sent); a draft waiting to start is forgotten. True when
    /// a session was open (its start went out, so its stop is due).
    pub fn end(&mut self, context: &TypingContext) -> bool {
        self.waiting.remove(context);
        self.sessions.remove(context).is_some()
    }

    /// When the start in `context` last went out, if a session is open.
    #[must_use]
    pub fn last_sent(&self, context: &TypingContext) -> Option<u64> {
        self.sessions.get(context).map(|s| s.last_sent_ms)
    }

    /// The start [`Self::input`] asked for in `context` was not sent:
    /// undo it, `sent_before` being [`Self::last_sent`] before that input.
    /// A refresh that failed leaves the session as its last real send left
    /// it, so the next input tries again; a first start that failed leaves
    /// no session (the next input starts one) and waits in
    /// [`Self::take_waiting`] for the host to retry it.
    pub fn unsent(&mut self, context: TypingContext, sent_before: Option<u64>) {
        if let Some(sent) = sent_before {
            if let Some(session) = self.sessions.get_mut(&context) {
                session.last_sent_ms = sent;
            }
        } else {
            self.sessions.remove(&context);
            self.waiting.insert(context);
        }
    }

    /// Take the waiting drafts `pick` selects: their starts are due now.
    pub fn take_waiting(&mut self, pick: impl Fn(&TypingContext) -> bool) -> Vec<TypingContext> {
        let taken: Vec<TypingContext> = self.waiting.iter().filter(|c| pick(c)).cloned().collect();
        for context in &taken {
            self.waiting.remove(context);
        }
        taken
    }

    /// Where the session in `context` stands at `now_ms`: ended once
    /// [`TYPING_IDLE_MS`] passed since its last input (it is removed).
    pub fn idle(&mut self, context: &TypingContext, now_ms: u64) -> Lapse {
        let Some(session) = self.sessions.get(context).copied() else {
            return Lapse::Gone;
        };
        let deadline = session.last_input_ms.saturating_add(TYPING_IDLE_MS);
        if deadline > now_ms {
            Lapse::Pending(deadline)
        } else {
            self.sessions.remove(context);
            Lapse::Ended
        }
    }

    /// Forget everything (lock).
    pub fn clear(&mut self) {
        self.sessions.clear();
        self.waiting.clear();
    }
}

#[cfg(test)]
#[path = "typing_tests.rs"]
mod tests;

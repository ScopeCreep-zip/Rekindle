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

use std::collections::HashMap;

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
}

impl TypingSessions {
    /// The user typed in `context` at `now_ms`.
    pub fn input(&mut self, context: TypingContext, now_ms: u64) -> Input {
        match self.sessions.get_mut(&context) {
            None => {
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
    /// message was sent). True when one was open.
    pub fn end(&mut self, context: &TypingContext) -> bool {
        self.sessions.remove(context).is_some()
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dm(peer: &str) -> TypingContext {
        TypingContext::Dm {
            peer_key: peer.to_owned(),
        }
    }

    #[test]
    fn a_start_lapses_after_its_ttl_without_a_refresh() {
        let mut expiry = TypingExpiry::default();
        assert!(expiry.start(dm("a"), "a".into(), 1_000, TYPING_TTL_MS));
        assert_eq!(expiry.lapse(&dm("a"), "a", 5_999), Lapse::Pending(6_000));
        assert_eq!(expiry.lapse(&dm("a"), "a", 6_000), Lapse::Ended);
        assert_eq!(expiry.lapse(&dm("a"), "a", 6_001), Lapse::Gone);
    }

    #[test]
    fn a_refresh_moves_the_deadline_and_is_not_news() {
        let mut expiry = TypingExpiry::default();
        assert!(expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS));
        assert!(!expiry.start(dm("a"), "a".into(), 3_000, TYPING_TTL_MS));
        assert_eq!(expiry.lapse(&dm("a"), "a", 5_000), Lapse::Pending(8_000));
    }

    #[test]
    fn a_stop_ends_the_start_and_its_timer_finds_nothing() {
        let mut expiry = TypingExpiry::default();
        expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS);
        assert!(expiry.stop(&dm("a"), "a"));
        assert!(!expiry.stop(&dm("a"), "a"));
        assert_eq!(expiry.lapse(&dm("a"), "a", 10_000), Lapse::Gone);
    }

    #[test]
    fn typists_are_kept_apart_by_context_and_name() {
        let mut expiry = TypingExpiry::default();
        let channel = TypingContext::Channel {
            community: "c".into(),
            channel: "general".into(),
        };
        expiry.start(dm("a"), "a".into(), 0, TYPING_TTL_MS);
        expiry.start(channel.clone(), "a".into(), 0, 10_000);
        expiry.start(channel.clone(), "b".into(), 0, 10_000);
        assert_eq!(expiry.lapse(&dm("a"), "a", 5_000), Lapse::Ended);
        assert_eq!(expiry.lapse(&channel, "a", 5_000), Lapse::Pending(10_000));
        assert!(expiry.stop(&channel, "b"));
        expiry.clear();
        assert_eq!(expiry.lapse(&channel, "a", 0), Lapse::Gone);
    }

    #[test]
    fn a_sender_starts_once_and_refreshes_while_input_continues() {
        let mut sessions = TypingSessions::default();
        assert_eq!(sessions.input(dm("b"), 0), Input::Started);
        assert_eq!(sessions.input(dm("b"), 1_000), Input::Quiet);
        assert_eq!(sessions.input(dm("b"), 2_999), Input::Quiet);
        assert_eq!(sessions.input(dm("b"), 3_000), Input::Refresh);
        assert_eq!(sessions.input(dm("b"), 4_000), Input::Quiet);
        assert_eq!(sessions.input(dm("b"), 6_000), Input::Refresh);
    }

    #[test]
    fn a_sender_stops_after_the_idle_time_from_its_last_input() {
        let mut sessions = TypingSessions::default();
        sessions.input(dm("b"), 0);
        sessions.input(dm("b"), 2_000);
        assert_eq!(sessions.idle(&dm("b"), 3_000), Lapse::Pending(5_000));
        assert_eq!(sessions.idle(&dm("b"), 5_000), Lapse::Ended);
        assert_eq!(sessions.idle(&dm("b"), 5_000), Lapse::Gone);
        assert_eq!(sessions.input(dm("b"), 6_000), Input::Started);
    }

    #[test]
    fn ending_a_session_leaves_its_idle_timer_nothing() {
        let mut sessions = TypingSessions::default();
        sessions.input(dm("b"), 0);
        assert!(sessions.end(&dm("b")));
        assert!(!sessions.end(&dm("b")));
        assert_eq!(sessions.idle(&dm("b"), 10_000), Lapse::Gone);
        sessions.input(dm("c"), 0);
        sessions.clear();
        assert_eq!(sessions.idle(&dm("c"), 0), Lapse::Gone);
    }
}

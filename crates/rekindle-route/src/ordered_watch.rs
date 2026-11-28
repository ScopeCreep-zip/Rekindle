//! A value followed by several tasks, each through a watch of its own,
//! sent in the order the followers subscribed (plan C9.2, RK1).
//!
//! One tokio watch with several waiters wakes them in an order tokio draws
//! from the runtime's own RNG (tokio 1.54 `sync/watch.rs`
//! `BigNotify::notified`, `thread_rng_n(8)`), not from any seed, so tasks
//! that each send or draw on waking did so in a per-process order. Here
//! each follower's watch has one waiter, and the sends run in one order,
//! so the woken tasks are queued in that order every run.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

/// Where each follower of a value gets a watch: a task that waits on the
/// value takes one of its own ([`follow`](Self::follow)), so waiters on an
/// [`OrderedWatch`] are woken in subscription order (RK1).
pub struct Follow<T>(Arc<dyn Fn() -> watch::Receiver<T> + Send + Sync>);

impl<T> Clone for Follow<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Clone + Send + Sync + 'static> Follow<T> {
    /// Followers that all share `rx`'s one watch (a source with no
    /// per-follower fan-out, e.g. the desktop's `network_ready`): tokio
    /// wakes its waiters in an order of its own ([module docs](self)).
    #[must_use]
    pub fn shared(rx: watch::Receiver<T>) -> Self {
        Self(Arc::new(move || rx.clone()))
    }
}

impl<T> Follow<T> {
    /// A watch of this follower's own.
    #[must_use]
    pub fn follow(&self) -> watch::Receiver<T> {
        (self.0)()
    }
}

/// The value and one watch per follower ([module docs](self)).
pub struct OrderedWatch<T> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T> {
    value: T,
    followers: Vec<watch::Sender<T>>,
}

impl<T: Default> Default for OrderedWatch<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner {
                value: T::default(),
                followers: Vec::new(),
            }),
        }
    }
}

impl<T: Clone> OrderedWatch<T> {
    /// A fan-out holding `value`, with no followers yet.
    pub fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(Inner {
                value,
                followers: Vec::new(),
            }),
        }
    }

    /// A new follower, holding the current value as seen (as
    /// [`watch::Sender::subscribe`]); it is sent each value after those
    /// that subscribed before it.
    pub fn subscribe(&self) -> watch::Receiver<T> {
        let mut inner = self.inner.lock();
        let (tx, rx) = watch::channel(inner.value.clone());
        inner.followers.push(tx);
        rx
    }

    /// Followers of this value that do not keep it alive: once it is
    /// dropped, a new follower's watch is closed (holding `T::default()`),
    /// as a watch whose sender is gone.
    #[must_use]
    pub fn follower(self: &Arc<Self>) -> Follow<T>
    where
        T: Default + Send + Sync + 'static,
    {
        let weak = Arc::downgrade(self);
        Follow(Arc::new(move || {
            weak.upgrade()
                .map_or_else(|| watch::channel(T::default()).1, |w| w.subscribe())
        }))
    }

    /// The current value.
    pub fn get(&self) -> T {
        self.inner.lock().value.clone()
    }

    /// Hold `value` and send it to every live follower, in subscription
    /// order (a follower whose receivers are all gone is dropped).
    pub fn send_replace(&self, value: T) {
        let mut inner = self.inner.lock();
        inner.followers.retain(|f| !f.is_closed());
        for follower in &inner.followers {
            follower.send_replace(value.clone());
        }
        inner.value = value;
    }

    /// [`send_replace`](Self::send_replace) only when `value` differs from
    /// the one held; whether it did.
    pub fn send_if_changed(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        if self.inner.lock().value == value {
            return false;
        }
        self.send_replace(value);
        true
    }
}

#[cfg(test)]
#[path = "ordered_watch_tests.rs"]
mod tests;

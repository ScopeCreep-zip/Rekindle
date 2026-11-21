//! A value followed by several tasks, each through a watch of its own,
//! sent in the order the followers subscribed (plan C9.2, RK1).
//!
//! One tokio watch with several waiters wakes them in an order tokio draws
//! from the runtime's own RNG (tokio 1.54 `sync/watch.rs`
//! `BigNotify::notified`, `thread_rng_n(8)`), not from any seed, so tasks
//! that each send or draw on waking did so in a per-process order. Here
//! each follower's watch has one waiter, and the sends run in one order,
//! so the woken tasks are queued in that order every run.

use parking_lot::Mutex;
use tokio::sync::watch;

/// The value and one watch per follower ([module docs](self)).
pub struct OrderedWatch<T> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T> {
    value: T,
    followers: Vec<watch::Sender<T>>,
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
mod tests {
    use super::OrderedWatch;

    #[tokio::test(flavor = "current_thread")]
    async fn followers_are_sent_in_subscription_order() {
        let watch = OrderedWatch::new(0u8);
        let order = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for i in 0..8 {
            let mut rx = watch.subscribe();
            let order = std::sync::Arc::clone(&order);
            tasks.push(tokio::spawn(async move {
                rx.changed().await.expect("sender alive");
                order.lock().push(i);
            }));
        }
        tokio::task::yield_now().await;
        watch.send_replace(1);
        for task in tasks {
            task.await.expect("a follower");
        }
        assert_eq!(*order.lock(), (0..8).collect::<Vec<_>>());
        assert!(!watch.send_if_changed(1));
        assert_eq!(watch.get(), 1);
    }
}

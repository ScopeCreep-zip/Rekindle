//! Which 1:1 call owns the running media (plan CE2, lead decision 30).
//!
//! One voice session runs at a time, and its SFrame keys, frame channels,
//! egress and ingress drivers and consent watch belong to one call. A
//! teardown asked for a call names it, and stops the media only when that
//! call owns it: a late `CallEnd` for a call the peer's new call superseded
//! never stops the new call's media (live CE2: the stale call's end shut the
//! whole session down after the new call's accept, and no media flowed).

use parking_lot::Mutex;

/// The call id whose media is running, if any.
#[derive(Debug, Default)]
pub struct CallMediaOwner(Mutex<Option<String>>);

impl CallMediaOwner {
    /// `call_id`'s media now runs (its voice session started).
    pub fn claim(&self, call_id: &str) {
        *self.0.lock() = Some(call_id.to_string());
    }

    /// Release the media if `call_id` owns it: `true` when it did, and the
    /// caller tears the media down; `false` when another call's media runs,
    /// or none, which then keeps running.
    pub fn release(&self, call_id: &str) -> bool {
        let mut owner = self.0.lock();
        if owner.as_deref() == Some(call_id) {
            *owner = None;
            true
        } else {
            false
        }
    }

    /// Whether `call_id`'s media is running.
    pub fn is(&self, call_id: &str) -> bool {
        self.0.lock().as_deref() == Some(call_id)
    }

    /// The call whose media is running.
    pub fn owner(&self) -> Option<String> {
        self.0.lock().clone()
    }

    /// The media stopped, whatever stopped it (a session-wide teardown:
    /// logout, the node detaching).
    pub fn clear(&self) {
        *self.0.lock() = None;
    }
}

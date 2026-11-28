//! Our beat's sealed `extras` (plan E3.4 "Contents, fixed size", S8-pre):
//! our `SessionExtras` (location, activity), filtered by our sharing
//! policy first, sealed under the community text key and stamped with its
//! generation (`SealedExtras`), inside the fixed 512 B beat. A reader opens
//! them with its current key, drops them on a generation it does not hold
//! (a removed member keeps the record key, never the rotated text key), and
//! keeps only the signals its own policy shares back.

use rekindle_codec::member_record::{RecordScope, SealedExtras};
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::presence::{MemberSession, PresenceSharingPolicy, SessionExtras};
use rekindle_utils::random::RandomSource;

use crate::community::policy::{filter_incoming, shared_session};

/// Our `session`'s extras as `policy` (our sharing consent, the Host's)
/// lets them out, for the beat at `at`, sealed under `key` (the current
/// community text key); `None` when the policy shares nothing or we hold no
/// key: extras are never written in the clear.
#[must_use]
pub fn seal_extras(
    session: MemberSession,
    policy: &PresenceSharingPolicy,
    key: Option<&MediaEncryptionKey>,
    at: RecordScope<'_>,
    rng: &dyn RandomSource,
) -> Option<SealedExtras> {
    let (_, extras) = shared_session(session, policy);
    let (extras, key) = (extras?, key?);
    match SealedExtras::seal(&extras, key, at, rng) {
        Ok(sealed) => Some(sealed),
        Err(error) => {
            tracing::warn!(%error, "beat extras not sealed");
            None
        }
    }
}

/// A member's `sealed` extras at `at`, opened with `key` (our current
/// community text key) and filtered by `our_policy`; `None` without a key,
/// on another generation, when they do not open, or when nothing is left.
#[must_use]
pub fn open_extras(
    sealed: &SealedExtras,
    key: Option<&MediaEncryptionKey>,
    at: RecordScope<'_>,
    our_policy: &PresenceSharingPolicy,
) -> Option<SessionExtras> {
    let extras = sealed.open(key?, at).ok()?;
    let session = filter_incoming(
        MemberSession {
            location: extras.location,
            activity: extras.activity,
            ..MemberSession::default()
        },
        our_policy,
    );
    (session.location.is_some() || session.activity.is_some()).then_some(SessionExtras {
        location: session.location,
        activity: session.activity,
    })
}

#[cfg(test)]
#[path = "extras_tests.rs"]
mod tests;

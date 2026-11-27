use parking_lot::Mutex;
use zeroize::ZeroizeOnDrop;

use crate::state::AppState;

/// Compiles only for a slot whose secret zeroizes when the slot is
/// cleared or replaced.
fn zeroizes_when_cleared<T: ZeroizeOnDrop>(_slot: &Mutex<Option<T>>) {}

/// Roadmap 1.9: logging out (`shell_session::ended::clear`) sets the
/// slot to `None`, which drops the secret; the slot's type is what
/// makes that drop wipe it.
#[test]
fn the_identity_secret_slot_zeroizes_on_clear() {
    let check: fn(&AppState) = |state| zeroizes_when_cleared(&state.identity_secret);
    let _ = check;
}

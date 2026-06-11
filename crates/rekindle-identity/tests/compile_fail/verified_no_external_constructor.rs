// Verified<T> must NOT be constructible from outside the crate.
// from_verified() is pub(crate), new_for_test() is #[cfg(test)] pub(crate).
// Neither is accessible from an external crate.
use rekindle_identity::Verified;

fn main() {
    let _v = Verified::<u64>::from_verified(42); // must not compile — pub(crate)
}

// PeerRef must NOT be constructible from outside the crate.
// anchor() is pub(crate), field is private, no From/Into.
use rekindle_identity::{PeerRef, IdentityRoot};

fn main() {
    let root = IdentityRoot::from_hex(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ).unwrap();
    let _peer = PeerRef::anchor(root); // must not compile — pub(crate)
}

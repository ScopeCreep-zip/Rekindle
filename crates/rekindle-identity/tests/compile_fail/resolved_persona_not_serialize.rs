// ResolvedPersona must NOT implement Serialize — it carries the PeerRef
// back-reference to the global identity. Serializing it would leak the
// pseudonym↔root linkage into a community wire context (D-08 violation).
use rekindle_identity::ResolvedPersona;

fn main() {
    // This type should not have serde::Serialize, so this should not compile.
    fn assert_serialize<T: serde::Serialize>() {}
    assert_serialize::<ResolvedPersona>(); // must not compile
}

// OriginSeed must NOT implement Serialize — never on a wire without vault encryption.
use rekindle_identity::OriginSeed;
use zeroize::Zeroizing;

fn main() {
    let seed = OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]));
    let _ = serde_json::to_string(&seed); // must not compile
}

// OriginSeed must NOT implement Clone — secret material cannot be duplicated.
use rekindle_identity::OriginSeed;
use zeroize::Zeroizing;

fn main() {
    let seed = OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]));
    let _copy = seed.clone(); // must not compile
}

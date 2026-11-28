//! The vault's passphrase stretch is pinned to a known answer, so a KDF
//! dependency change that derived different bytes for the same stored
//! parameters (every existing vault refusing its own passphrase) fails
//! here, not at a user's unlock.

use super::VaultHeader;

#[test]
fn the_passphrase_stretch_matches_its_known_answer() {
    let header = VaultHeader {
        m_cost_kib: 1024,
        t_cost: 2,
        p_cost: 1,
        output_len: 32,
        salt: [7u8; 32],
        duress_salt: [0u8; 32],
        duress_tag: [0u8; 32],
    };
    let out = header.stretch("correct horse battery staple", &[7u8; 32]).unwrap();
    assert_eq!(hex::encode(&*out), "28ce213c03186b8a6ecefe8d9689e4cc858f48c43e18d316e786335e1e73f872");
}

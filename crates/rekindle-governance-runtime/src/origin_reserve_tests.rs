use rekindle_governance::genesis::creator_accept_signing_bytes;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};

use super::*;

fn accept(seed: u8, anchor: &str) -> CoCreatorAcceptance {
    let key = SigningKey::from_bytes(&[seed; 32]);
    let pseudonym = PseudonymKey(key.verifying_key().to_bytes());
    let signature = key
        .sign(&creator_accept_signing_bytes(anchor, &pseudonym))
        .to_bytes();
    CoCreatorAcceptance {
        pseudonym,
        signature,
    }
}

/// Decision (16): the cert lists exactly the verified acceptances, in
/// order; one signed for another anchor refuses the whole genesis.
#[test]
fn only_verified_acceptances_become_further_creators() {
    let (b, c) = (accept(2, "VLD0:a"), accept(3, "VLD0:a"));
    assert_eq!(
        verified_creators("VLD0:a", &[b.clone(), c.clone()]).unwrap(),
        vec![b.pseudonym.clone(), c.pseudonym.clone()]
    );
    let elsewhere = accept(4, "VLD0:other");
    assert!(matches!(
        verified_creators("VLD0:a", &[b, elsewhere]),
        Err(GovernanceRuntimeError::Crypto(_))
    ));
}

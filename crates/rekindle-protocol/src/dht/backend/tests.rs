use rekindle_records::port::{PortError, RecordSchema, SchemaMember, SetOutcome};
use rekindle_types::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};
use veilid_core::{
    BarePublicKey, BareSecretKey, KeyPair, PublicKey, SafetySelection, Sequencing, Stability,
    ValueSeqNum, VeilidAPIError, CRYPTO_KIND_VLD0,
};

use super::{dht_error, dht_schema, parse_keypair, safety_selection};
use crate::dht::pool::{set_failed, to_port};
use crate::ProtocolError;

/// The Veilid-free keypair text (`rekindle_crypto::keypair_text`, plan
/// C9.1b) is exactly what Veilid prints and parses.
#[test]
fn keypair_text_is_veilids() {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
    let public = signing.verifying_key().to_bytes();
    let secret = signing.to_bytes();
    let veilid = KeyPair::new_from_parts(
        PublicKey::new(CRYPTO_KIND_VLD0, BarePublicKey::new(&public)),
        BareSecretKey::new(&secret),
    );
    let text = rekindle_crypto::keypair_text::keypair_text(&public, &secret);
    assert_eq!(veilid.to_string(), text);
    assert_eq!(parse_keypair(&text).unwrap(), veilid);
    assert_eq!(
        rekindle_crypto::keypair_text::keypair_text_from_signing(&signing),
        text
    );
}

/// The fact every occupancy check rests on: Veilid's first write to a
/// subkey lands at seq 0, so `Some(0)` is occupied, not empty
/// (`record_report` maps through `to_option`). Two readers once
/// flattened `None` to 0 and tested `!= 0`, classifying every write-once
/// member as absent.
#[test]
fn seq_zero_is_written_not_empty() {
    let seqs = [
        ValueSeqNum::NONE,
        ValueSeqNum::from(0),
        ValueSeqNum::from(1),
        ValueSeqNum::NONE,
    ];
    let mapped: Vec<Option<u32>> = seqs.iter().map(ValueSeqNum::to_option).collect();
    assert_eq!(mapped, vec![None, Some(0), Some(1), None]);
}

/// The port's schema and Veilid's agree on every subkey count.
#[test]
fn schemas_keep_their_subkey_counts() {
    let dflt = RecordSchema::Dflt { owner_subkeys: 9 };
    assert_eq!(
        dht_schema(&dflt).unwrap().max_subkey() as usize + 1,
        dflt.subkey_count()
    );
    let smpl = RecordSchema::Smpl {
        owner_subkeys: 0,
        members: (1u8..=3)
            .map(|b| SchemaMember {
                key: [b; 32],
                subkeys: 1,
            })
            .collect(),
    };
    assert_eq!(
        dht_schema(&smpl).unwrap().max_subkey() as usize + 1,
        smpl.subkey_count()
    );
    assert!(dht_schema(&RecordSchema::Dflt { owner_subkeys: 0 }).is_err());
}

#[test]
fn the_safety_selection_never_drops_below_the_anonymity_floor() {
    let mut profile = SafetyProfile::default_dht();
    profile.hop_count = 1;
    let SafetySelection::Safe(spec) = safety_selection(&profile) else {
        panic!("always Safe");
    };
    assert_eq!(spec.hop_count, usize::from(ANONYMITY_HOP_FLOOR));
    profile.hop_count = 5;
    profile.stability = StabilityPreference::LowLatency;
    profile.sequencing = SequencingPreference::NoPreference;
    let SafetySelection::Safe(spec) = safety_selection(&profile) else {
        panic!("always Safe");
    };
    assert_eq!(spec.hop_count, 5);
    assert_eq!(spec.stability, Stability::LowLatency);
    assert_eq!(spec.sequencing, Sequencing::PreferUnordered);
}

/// Every Veilid error kind, once.
fn every_kind() -> Vec<VeilidAPIError> {
    use base64::Engine;
    let text = format!(
        "VLD0:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode((1..=32).collect::<Vec<u8>>())
    );
    let key = crate::dht::parse_record_key(&text).expect("a record key");
    vec![
        VeilidAPIError::NotInitialized,
        VeilidAPIError::AlreadyInitialized,
        VeilidAPIError::Timeout,
        VeilidAPIError::try_again("busy"),
        VeilidAPIError::InvalidTarget {
            message: "t".into(),
        },
        VeilidAPIError::NoConnection {
            message: "c".into(),
        },
        VeilidAPIError::Shutdown,
        VeilidAPIError::key_not_found(key.opaque()),
        VeilidAPIError::Internal {
            message: "i".into(),
        },
        VeilidAPIError::Unimplemented {
            message: "u".into(),
        },
        VeilidAPIError::ParseError {
            message: "p".into(),
            value: "v".into(),
        },
        VeilidAPIError::InvalidArgument {
            context: "x".into(),
            argument: "a".into(),
            value: "v".into(),
        },
        VeilidAPIError::MissingArgument {
            context: "x".into(),
            argument: "a".into(),
        },
        VeilidAPIError::generic("failed schema validation"),
        VeilidAPIError::TransactionNotFound {
            message: "n".into(),
        },
    ]
}

/// The pool's classification before the backend seam, on `VeilidAPIError`.
fn classified_before(e: &VeilidAPIError) -> ProtocolError {
    match e {
        VeilidAPIError::Shutdown => ProtocolError::PoolClosed,
        VeilidAPIError::TryAgain { .. }
        | VeilidAPIError::KeyNotFound { .. }
        | VeilidAPIError::TransactionNotFound { .. } => {
            ProtocolError::DhtRecordUnreachable(e.to_string())
        }
        _ => ProtocolError::DhtError(e.to_string()),
    }
}

fn set_before(e: &VeilidAPIError) -> Result<SetOutcome, PortError> {
    match e {
        VeilidAPIError::TryAgain { .. } => Ok(SetOutcome::Offline),
        other => Err(classified_before(other).into()),
    }
}

/// Through the backend, every kind classifies as it did on Veilid's error:
/// the same port error with the same text, the same set outcome.
#[test]
fn every_error_kind_classifies_as_before() {
    for e in every_kind() {
        let through = dht_error(&e);
        assert_eq!(through.to_string(), e.to_string(), "{e:?}");
        assert_eq!(
            to_port(&through),
            PortError::from(classified_before(&e)),
            "{e:?}"
        );
        assert_eq!(set_failed(&through), set_before(&e), "{e:?}");
    }
}

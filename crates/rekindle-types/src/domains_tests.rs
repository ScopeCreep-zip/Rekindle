use super::{is_well_formed, ALL, IDENTITY_KEY_SIGNATURES, PRE_FORMAT};

/// The identity key signs under each of these and nothing else; every
/// one is distinct, none is a prefix of another, and each is in the
/// registry.
#[test]
fn identity_key_signing_domains_are_distinct_and_prefix_free() {
    for (i, a) in IDENTITY_KEY_SIGNATURES.iter().enumerate() {
        assert!(ALL.iter().any(|(_, l)| l == a), "`{a}` is not registered");
        for (j, b) in IDENTITY_KEY_SIGNATURES.iter().enumerate() {
            if i != j {
                assert!(!b.starts_with(a), "`{a}` is a prefix of `{b}`");
            }
        }
    }
}

#[test]
fn every_label_is_unique() {
    for (i, (name_a, a)) in ALL.iter().enumerate() {
        for (name_b, b) in &ALL[i + 1..] {
            assert_ne!(a, b, "{name_a} and {name_b} share the label `{a}`");
        }
    }
}

/// Labels are often concatenated with the data they scope; if one
/// label were a prefix of another, `A ‖ x` and `B ‖ y` could collide.
#[test]
fn no_label_is_a_prefix_of_another() {
    for (name_a, a) in ALL {
        for (name_b, b) in ALL {
            if name_a != name_b {
                assert!(
                    !b.starts_with(a),
                    "{name_a} (`{a}`) is a prefix of {name_b} (`{b}`)"
                );
            }
        }
    }
}

#[test]
fn labels_follow_the_naming_rule_or_are_scheduled_for_removal() {
    for (name, label) in ALL {
        assert!(
            is_well_formed(label) || PRE_FORMAT.contains(label),
            "{name} = `{label}` must be `rekindle-<purpose>-v<N>`"
        );
        if PRE_FORMAT.contains(label) {
            assert!(
                !is_well_formed(label),
                "{name} is well-formed; drop it from PRE_FORMAT"
            );
        }
    }
}

#[test]
fn naming_rule() {
    assert!(is_well_formed("rekindle-gov-op-v1"));
    assert!(is_well_formed("rekindle-voice-packet-v12"));
    assert!(!is_well_formed("rekindle-gov-op"));
    assert!(!is_well_formed("rekindle--v1"));
    assert!(!is_well_formed("Rekindle-x-v1"));
    assert!(!is_well_formed("rekindle-x-v"));
    assert!(!is_well_formed("rekindle v1 vault-entry-gcm"));
}

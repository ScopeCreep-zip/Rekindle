use super::*;
use crate::{builtin, validate};

#[test]
fn the_login_copy_keeps_no_identifying_field_or_sound_and_still_validates() {
    let source = validate::run(builtin::PACKS[0].bytes).unwrap();
    let stripped = strip(&source.bytes).unwrap();
    assert_ne!(stripped.hash, source.hash, "re-hashed");
    let pkg = package::read(&stripped.bytes).unwrap();
    let m = crate::manifest::parse(&pkg).unwrap();
    assert_eq!(m.id, LOGIN_ID);
    assert_eq!(m.name, LOGIN_NAME);
    assert!(m.description.is_empty());
    assert!(m.source.is_none() && m.author.is_none() && m.signature.is_none());
    assert!(m.sounds.is_empty());
    assert!(pkg.files().all(|(name, _)| !name.starts_with("sounds/")));
    // What the login window draws passes the same pipeline.
    let again = validate::run(&stripped.bytes).unwrap();
    assert_eq!(again.bytes, stripped.bytes, "a fixed point");
}

#[test]
fn a_sound_bank_loses_every_cue() {
    let bank = builtin_bank();
    let source = package::read(bank).unwrap();
    assert!(source.files().any(|(n, _)| n.starts_with("sounds/")));
    let stripped = strip(bank).unwrap();
    let pkg = package::read(&stripped.bytes).unwrap();
    assert!(pkg.files().all(|(n, _)| !n.starts_with("sounds/")));
}

fn builtin_bank() -> &'static [u8] {
    include_bytes!("../builtin/defaults.rkskin")
}

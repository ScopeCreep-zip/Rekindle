use rekindle_secrets::ed25519_dalek::{Signer as _, SigningKey};
use rekindle_types::group_dm::{GroupDmRosterMember, MAX_MEMBERS, MAX_NAME_CHARS};
use rekindle_utils::OsRandom;

use super::*;
use crate::sender_key;

pub(crate) fn signer(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}

pub(crate) fn member_of(key: &SigningKey) -> GroupDmRosterMember {
    let identity = hex::encode(key.verifying_key().to_bytes());
    GroupDmRosterMember {
        profile_record_key: format!("VLD0:profile-{}", &identity[..8]),
        participant_record_key: format!("VLD0:chain-{}", &identity[..8]),
        identity,
    }
}

fn members(n: u8) -> Vec<GroupDmRosterMember> {
    (1..=n).map(|i| member_of(&signer(i))).collect()
}

#[test]
fn roster_bounds() {
    assert!(matches!(
        Roster::new(members(2)),
        Err(DmError::InvalidRoster(_))
    ));
    assert!(Roster::new(members(3)).is_ok());
    let most = u8::try_from(MAX_MEMBERS).expect("fits");
    assert!(Roster::new(members(most)).is_ok());
    assert!(Roster::new(members(most + 1)).is_err());
    let mut repeated = members(3);
    repeated.push(repeated[0].clone());
    assert!(Roster::new(repeated).is_err());
    let mut shared_chain = members(3);
    shared_chain[2].participant_record_key = shared_chain[1].participant_record_key.clone();
    assert!(Roster::new(shared_chain).is_err());
    let mut not_identity = members(3);
    not_identity[1].identity = "ABC".into();
    assert!(Roster::new(not_identity).is_err());
    let small = Roster::from_stored(members(2)).expect("a shrunk roster");
    assert!(small
        .without(&small.members()[0].identity.clone())
        .is_some());
    assert!(small.without("nobody").is_none());
}

#[test]
fn name_bounds() {
    assert!(check_name("").is_err());
    assert!(check_name("   ").is_err());
    assert!(check_name(&"é".repeat(MAX_NAME_CHARS)).is_ok());
    assert!(check_name(&"a".repeat(MAX_NAME_CHARS + 1)).is_err());
}

fn signed(creator: &SigningKey) -> GroupDmInvite {
    let mut list = vec![member_of(creator)];
    list.extend(members(3).into_iter().skip(1));
    let roster = Roster::new(list).expect("roster");
    let key = sender_key::mint_first(&OsRandom);
    build_invite([9; 16], "squad", &roster, &key, |m| {
        creator.sign(m).to_bytes()
    })
}

#[test]
fn invite_signature_verifies_and_tampering_fails() {
    let creator = signer(1);
    let invite = signed(&creator);
    let from = member_of(&creator).identity;
    let roster = verify_invite(&invite, &from).expect("verifies");
    assert_eq!(roster.creator().identity, from);

    let mut tampered = invite.clone();
    tampered.roster[2] = member_of(&signer(7));
    assert!(matches!(
        verify_invite(&tampered, &from),
        Err(DmError::BadSignature)
    ));
    let mut renamed = invite.clone();
    renamed.name = "other".into();
    assert!(matches!(
        verify_invite(&renamed, &from),
        Err(DmError::BadSignature)
    ));
    let mut re_epoched = invite.clone();
    re_epoched.epoch += 1;
    assert!(matches!(
        verify_invite(&re_epoched, &from),
        Err(DmError::BadSignature)
    ));

    // Only the creator's session carries its invite.
    let other = member_of(&signer(2)).identity;
    assert!(matches!(
        verify_invite(&invite, &other),
        Err(DmError::NotOnRoster(_))
    ));
    // A member who re-signs a roster naming someone else as creator fails.
    let forged = build_invite(
        [9; 16],
        "squad",
        &Roster::new(invite.roster.clone()).expect("r"),
        &sender_key::mint_first(&OsRandom),
        |m| signer(2).sign(m).to_bytes(),
    );
    assert!(matches!(
        verify_invite(&forged, &from),
        Err(DmError::BadSignature)
    ));
}

#[test]
fn epochs_are_monotonic() {
    let k0 = GroupSenderKey::mint(0, &OsRandom);
    let k1 = k0.next(&OsRandom).expect("next");
    assert_eq!(key_update(None, &k0).expect("first"), KeyUpdate::Store);
    assert_eq!(key_update(Some(&k0), &k1).expect("up"), KeyUpdate::Store);
    assert_eq!(
        key_update(Some(&k1), &k1).expect("dup"),
        KeyUpdate::Duplicate
    );
    assert!(matches!(
        key_update(Some(&k1), &k0),
        Err(DmError::StaleEpoch { got: 0, current: 1 })
    ));
    let other1 = GroupSenderKey::mint(1, &OsRandom);
    assert!(matches!(
        key_update(Some(&k1), &other1),
        Err(DmError::ConflictingKey(1))
    ));
    assert!(check_frame_epoch("ann", 1, 1).is_ok());
    assert!(matches!(
        check_frame_epoch("ann", 0, 1),
        Err(DmError::StaleEpoch { .. })
    ));
    assert!(matches!(
        check_frame_epoch("ann", 2, 1),
        Err(DmError::KeyMissing { epoch: 2, .. })
    ));
}

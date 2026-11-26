use super::*;
use crate::capnp_codec::direct::{encode_root, DirectBody, DmTypeId};

fn key(c: char) -> String {
    format!("VLD0:{}", c.to_string().repeat(43))
}

/// Plan E3.2 S7s.3: a `CreatorJoin` whose invite is not a link, or is
/// a link to another community, is refused on decode, so a co-creator
/// never joins a community other than the one it accepted.
#[test]
fn a_creator_join_names_its_own_anchor() {
    let bytes = |invite: &str| {
        encode_root::<creator_join::Owned>(|mut b| {
            b.set_anchor(key('A').as_str());
            b.set_invite(invite);
        })
    };
    let link = |anchor: &str| {
        format!(
            "rekindle://invite/{anchor}/{}/{}/{}/7/3",
            key('S'),
            "ab".repeat(16),
            "cd".repeat(32)
        )
    };
    assert!(DirectBody::decode(DmTypeId::CreatorJoin, &bytes(&link(&key('B')))).is_err());
    assert!(DirectBody::decode(DmTypeId::CreatorJoin, &bytes("hello")).is_err());
    assert!(DirectBody::decode(DmTypeId::CreatorJoin, &bytes(&link(&key('A')))).is_ok());
}

/// A pseudonym or signature of the wrong length, or a name with a
/// control character, is refused.
#[test]
fn bad_creator_bodies_are_refused() {
    let accept = encode_root::<creator_accept::Owned>(|mut b| {
        b.set_anchor(key('A').as_str());
        b.set_pseudonym(&[1; 31]);
        b.set_signature(&[2; 64]);
    });
    assert!(DirectBody::decode(DmTypeId::CreatorAccept, &accept).is_err());
    let invite = encode_root::<creator_invite::Owned>(|mut b| {
        b.set_anchor(key('A').as_str());
        b.set_community_name("a\u{1b}b");
    });
    assert!(DirectBody::decode(DmTypeId::CreatorInvite, &invite).is_err());
}

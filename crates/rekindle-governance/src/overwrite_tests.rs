use super::*;

#[test]
fn a_role_target_is_its_32_hex_and_a_member_its_64() {
    let everyone = OverwriteTarget::Role(RoleId::EVERYONE);
    assert_eq!(everyone.key(), "0".repeat(32));
    assert_eq!(
        OverwriteTarget::parse("role", &"0".repeat(32)),
        Some(everyone.clone())
    );
    let member = OverwriteTarget::Member(PseudonymKey([0xab; 32]));
    assert_eq!(member.key(), "ab".repeat(32));
    assert_eq!(
        OverwriteTarget::parse("member", &"ab".repeat(32)),
        Some(member.clone())
    );
}

#[test]
fn a_malformed_or_non_canonical_target_is_refused() {
    for (ty, id) in [
        ("role", format!("{:?}", RoleId::EVERYONE)),
        ("role", "AB".repeat(16)),
        ("role", "ab".repeat(32)),
        ("role", String::new()),
        ("member", "ab".repeat(16)),
        ("member", "zz".repeat(32)),
        ("everyone", "0".repeat(32)),
    ] {
        assert_eq!(OverwriteTarget::parse(ty, &id), None, "{ty} {id}");
    }
}

use super::*;

#[test]
fn administrator_bypasses_all() {
    let perms = Permissions(ADMINISTRATOR);
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(BAN_MEMBERS));
    assert!(perms.has(MANAGE_CHANNELS));
}

#[test]
fn specific_permission_check() {
    let perms = Permissions(SEND_MESSAGES | ADD_REACTIONS);
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(ADD_REACTIONS));
    assert!(!perms.has(BAN_MEMBERS));
}

#[test]
fn default_everyone_includes_basics() {
    let perms = Permissions(DEFAULT_EVERYONE);
    assert!(perms.has(VIEW_CHANNELS));
    assert!(perms.has(SEND_MESSAGES));
    assert!(perms.has(READ_HISTORY));
    assert!(perms.has(CONNECT));
    assert!(perms.has(SPEAK));
    assert!(!perms.has(BAN_MEMBERS));
    assert!(!perms.has(ADMINISTRATOR));
}

#[test]
fn catalogue_covers_each_defined_bit_once() {
    let mut seen = 0;
    for entry in PERMISSION_CATALOGUE
        .iter()
        .flat_map(|group| group.permissions)
    {
        assert!(entry.bit.is_power_of_two(), "{}", entry.key);
        assert_eq!(seen & entry.bit, 0, "duplicate bit for {}", entry.key);
        seen |= entry.bit;
    }
    assert_eq!(seen, DEFINED);
}

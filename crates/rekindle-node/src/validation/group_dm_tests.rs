use super::*;

fn key(digit: char) -> String {
    std::iter::repeat_n(digit, 64).collect()
}

#[test]
fn create_bounds_name_and_members() {
    let two = vec![key('a'), key('b')];
    assert!(create("raid", &two, None).is_ok());
    assert!(create("  ", &two, None).is_err());
    assert!(create(&"x".repeat(MAX_NAME_CHARS + 1), &two, None).is_err());
    assert!(create("raid", &[key('a')], None).is_err());
    assert!(create("raid", &vec![key('a'); 2], None).is_err());
    let eight: Vec<String> = "abcdef01".chars().map(key).collect();
    assert!(create("raid", &eight, None).is_err());
    assert!(create("raid", &eight[..7], None).is_ok());
    assert!(create("raid", &two, Some(&key('a'))).is_ok());
    assert!(create("raid", &two, Some(&key('c'))).is_err());
}

#[test]
fn a_rename_names_a_group_within_the_name_bounds() {
    let rename = |group_id: String, name: &str| {
        check(&IpcRequest::GroupDm(GroupDmRequest::Rename {
            group_id,
            name: name.to_owned(),
        }))
    };
    let g = "0f".repeat(16);
    assert!(rename(g.clone(), "scrim night").is_ok());
    assert!(rename(g.clone(), &"x".repeat(MAX_NAME_CHARS)).is_ok());
    assert!(rename(g.clone(), &"x".repeat(MAX_NAME_CHARS + 1)).is_err());
    assert!(rename(g, "   ").is_err());
    assert!(rename("0F".repeat(16), "scrim night").is_err());
}

#[test]
fn an_invite_names_a_group_and_one_to_seven_distinct_keys() {
    let g = "0f".repeat(16);
    let seven: Vec<String> = "abcdef0".chars().map(key).collect();
    assert!(invite(&g, &seven[..1]).is_ok());
    assert!(invite(&g, &seven).is_ok());
    let mut eight = seven.clone();
    eight.push(key('1'));
    assert!(invite(&g, &eight).is_err());
    assert!(invite(&g, &[]).is_err());
    assert!(invite(&g, &vec![key('a'); 2]).is_err());
    assert!(invite(&g, &["nope".to_owned()]).is_err());
    assert!(invite(&"0F".repeat(16), &seven[..1]).is_err());
}

#[test]
fn group_ids_are_sixteen_bytes_of_lowercase_hex() {
    assert!(group_id(&"0f".repeat(16)).is_ok());
    assert!(group_id(&"0F".repeat(16)).is_err());
    assert!(group_id(&"0f".repeat(15)).is_err());
}

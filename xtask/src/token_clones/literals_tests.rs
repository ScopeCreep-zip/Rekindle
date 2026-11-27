use super::{literals_of, string_values};

#[test]
fn sql_strings_match_by_value() {
    let ordinary = literals_of(r#""SELECT name FROM users WHERE id = ?1""#, "rust");
    let raw = literals_of(r##"r#"SELECT name FROM users WHERE id = ?1"#"##, "rust");
    let different = literals_of(r#""SELECT name FROM teams WHERE id = ?1""#, "rust");
    assert!(string_values(&ordinary, 1, 1).eq(string_values(&raw, 1, 1)));
    assert!(!string_values(&ordinary, 1, 1).eq(string_values(&different, 1, 1)));
}

#[test]
fn quotes_in_comments_have_no_value() {
    let strings = literals_of(
        "// 'ignored'\nconst name = 'card'; /* \"ignored\" */",
        "typescript",
    );
    assert_eq!(
        string_values(&strings, 1, 2).collect::<Vec<_>>(),
        vec![b"card".as_slice()]
    );
}

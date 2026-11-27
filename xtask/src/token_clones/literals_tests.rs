use super::{literals_of, string_values};

#[test]
fn sql_strings_match_by_value() {
    let ordinary = literals_of(r#""SELECT name FROM users WHERE id = ?1""#, "rust");
    let raw = literals_of(r##"r#"SELECT name FROM users WHERE id = ?1"#"##, "rust");
    let different = literals_of(r#""SELECT name FROM teams WHERE id = ?1""#, "rust");
    assert!(string_values(&ordinary, 0..usize::MAX).eq(string_values(&raw, 0..usize::MAX)));
    assert!(!string_values(&ordinary, 0..usize::MAX).eq(string_values(&different, 0..usize::MAX)));
}

#[test]
fn quotes_in_comments_have_no_value() {
    let strings = literals_of(
        "// 'ignored'\nconst name = 'card'; /* \"ignored\" */",
        "typescript",
    );
    assert_eq!(
        string_values(&strings, 0..usize::MAX).collect::<Vec<_>>(),
        vec![b"card".as_slice()]
    );
}

#[test]
fn adjacent_literals_on_the_same_line_stay_outside_the_match() {
    let a = r#"read("matched"); read("first");"#;
    let b = r#"read("matched"); read("second");"#;
    let match_span = 0..a.find(';').unwrap();
    let (left, right) = (literals_of(a, "rust"), literals_of(b, "rust"));
    assert!(string_values(&left, match_span.clone()).eq(string_values(&right, match_span)));
    assert!(!string_values(&left, 0..a.len()).eq(string_values(&right, 0..b.len())));
}

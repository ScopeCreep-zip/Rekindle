use super::*;

#[test]
fn blank_returns_none() {
    assert!(build_match_expr("   ").is_none());
    assert!(build_match_expr("").is_none());
}

#[test]
fn plain_words_become_quoted_and_anded() {
    assert_eq!(
        build_match_expr("hello world"),
        Some(r#""hello" "world""#.to_string())
    );
}

#[test]
fn quotes_inside_token_are_doubled() {
    assert_eq!(
        build_match_expr(r#"say"hi"#),
        Some(r#""say""hi""#.to_string())
    );
}

#[test]
fn collapses_extra_whitespace() {
    assert_eq!(
        build_match_expr("  one   two   "),
        Some(r#""one" "two""#.to_string())
    );
}

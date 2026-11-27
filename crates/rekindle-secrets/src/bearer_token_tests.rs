use super::BearerToken;

#[test]
fn minted_tokens_are_distinct_and_match_only_themselves() {
    let (a, b) = (BearerToken::mint(), BearerToken::mint());
    assert_eq!(a.as_str().len(), 52);
    assert_ne!(a.as_str(), b.as_str());
    assert!(a.matches(a.as_str().as_bytes()));
    assert!(!a.matches(b.as_str().as_bytes()));
    assert!(!a.matches(&a.as_str().as_bytes()[..51]));
    assert_eq!(format!("{a:?}"), "BearerToken(<redacted>)");
}

#[test]
fn parse_accepts_only_a_minted_spelling() {
    let a = BearerToken::mint();
    let parsed = BearerToken::parse(a.as_str()).expect("a minted token parses");
    assert!(parsed.matches(a.as_str().as_bytes()));
    assert!(BearerToken::parse(&a.as_str()[..51]).is_none());
    assert!(BearerToken::parse(&a.as_str().to_ascii_uppercase()).is_none());
    assert!(BearerToken::parse(&format!("{}a", a.as_str())).is_none());
    assert!(BearerToken::parse("").is_none());
    assert!(BearerToken::parse(&"1".repeat(52)).is_none());
}

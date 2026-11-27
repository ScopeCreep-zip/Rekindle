use super::*;

const PAGE: &str = "https://web.archive.org/web/2009/http://www.xfire.com/downloads/1032/";
const QUOTE: &str = "The Combat skin is an official skin created by Xfire.";

#[test]
fn credits_are_bounded_and_checked() {
    let c = XfireCredit::new(PAGE, QUOTE).unwrap();
    assert_eq!((c.source(), c.quote()), (PAGE, QUOTE));
    assert!(XfireCredit::new("ftp://x/y", QUOTE).is_err());
    assert!(XfireCredit::new("https://a b", QUOTE).is_err());
    assert!(XfireCredit::new(PAGE, "").is_err());
    assert!(XfireCredit::new(PAGE, "An official skin.").is_err());
    assert!(XfireCredit::new(PAGE, "Xfire\u{202E} made it").is_err());
    assert!(XfireCredit::new(PAGE, "Xfire\nmade it").is_err());
    assert!(XfireCredit::new(PAGE, &format!("Xfire{}", "a".repeat(MAX_QUOTE))).is_err());
    let long = format!("https://{}", "a".repeat(MAX_SOURCE));
    assert!(XfireCredit::new(&long, QUOTE).is_err());
}

#[test]
fn a_credit_reaches_both_texts() {
    let c = XfireCredit::new(PAGE, QUOTE).unwrap();
    let a = Authorship::XfireByCredit(&c);
    for text in [a.manifest_text(), a.notice_text()] {
        assert!(text.contains(PAGE) && text.contains(QUOTE), "{text}");
        assert!(text.contains("2026-10-15"), "{text}");
        assert!(!text.contains("Third-party"), "{text}");
    }
    assert!(Authorship::ThirdParty
        .manifest_text()
        .starts_with("Third-party"));
}

use super::*;

fn tree(files: &[(&str, &[u8])]) -> Tree {
    files
        .iter()
        .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
        .collect()
}

#[test]
fn names_are_trimmed_bounded_and_unique() {
    let long = "a".repeat(70);
    let raw = [" Ann ", "", "Ann", long.as_str(), "Bo"];
    let got = names(raw, &["Bo".to_owned()]);
    assert_eq!(got, vec!["Ann".to_owned(), "a".repeat(64)]);
    let many: Vec<String> = (0..20).map(|i| format!("n{i}")).collect();
    assert_eq!(
        names(many.iter().map(String::as_str), &[]).len(),
        MAX_CREDIT_NAMES
    );
}

#[test]
fn only_http_urls_are_source_links() {
    assert_eq!(
        url(" http://www.xfire.com/ ").as_deref(),
        Some("http://www.xfire.com/")
    );
    assert_eq!(url("www.xfire.com"), None);
    assert_eq!(url("javascript:alert(1)"), None);
    assert_eq!(url("https://a b"), None);
}

#[test]
fn a_licence_file_is_read_before_a_readme() {
    assert_eq!(
        licence(&tree(&[("a.ini", b"x")])),
        OriginalLicence::NoneStated
    );
    let t = tree(&[
        ("ReadMe.txt", b"By Ann.\r\nAll rights reserved.\r\n"),
        ("sub/license.txt", b"not the root"),
    ]);
    assert_eq!(
        licence(&t),
        OriginalLicence::Text("By Ann.\nAll rights reserved.".into())
    );
    let t = tree(&[
        ("LICENSE.TXT", b"Free\tto share \x93as is\x94."),
        ("readme.txt", b"r"),
    ]);
    assert_eq!(
        licence(&t),
        OriginalLicence::Text("Free    to share \u{201C}as is\u{201D}.".into())
    );
    assert_eq!(
        licence(&tree(&[("readme.txt", b"\r\n ")])),
        OriginalLicence::NoneStated
    );
}

#[test]
fn a_credit_names_xfire_and_is_the_link_when_the_skin_names_none() {
    let c = XfireCredit::new("https://example.org/combat", "Xfire made Combat.").unwrap();
    let t = tree(&[]);
    let got = build(&t, &[], &[], None, Some(&c));
    assert_eq!(got.authors, vec![XFIRE_AUTHOR.to_owned()]);
    assert_eq!(got.url.as_deref(), Some("https://example.org/combat"));
    assert_eq!(got.licence, OriginalLicence::NoneStated);
    let stated = build(
        &t,
        &[XFIRE_AUTHOR.to_owned()],
        &[XFIRE_AUTHOR.to_owned(), "Tim Brengle".to_owned()],
        Some("http://www.xfire.com/"),
        Some(&c),
    );
    assert_eq!(stated.authors, vec![XFIRE_AUTHOR.to_owned()]);
    assert_eq!(stated.contributors, vec!["Tim Brengle".to_owned()]);
    assert_eq!(stated.url.as_deref(), Some("http://www.xfire.com/"));
}

use super::*;

#[test]
fn reads_and_refuses_entities() {
    let n = parse(
        "t",
        b"<?xml version=\"1.0\"?><!DOCTYPE A SYSTEM \"a.dtd\"><A x=\"1\"><B> hi </B><C/></A>",
    )
    .unwrap();
    assert_eq!(n.name, "A");
    assert_eq!(n.attr("x"), Some("1"));
    assert_eq!(n.attr_ci("X"), Some("1"));
    assert_eq!(n.attr("X"), None);
    assert_eq!(n.children[0].text, "hi");
    assert!(parse("t", b"<A>&ext;</A>").is_err());
    assert!(parse("t", b"<A></A><B/>").is_err());
}

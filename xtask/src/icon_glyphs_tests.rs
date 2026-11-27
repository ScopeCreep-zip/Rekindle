use super::parse_icon_line;

#[test]
fn parses_the_declared_shape() {
    let l =
        parse_icon_line(r#"export const ICON_X = "\u{F03E2}";  // nf-md-paperclip (attach file)"#)
            .ok();
    assert_eq!(
        l.map(|l| (l.name, l.codepoint, l.glyph)),
        Some(("ICON_X".into(), 0xF03E2, "md-paperclip".into()))
    );
    let dash =
        parse_icon_line(r#"export const ICON_Y = "\u{F0415}"; // nf-md-plus-circle — x"#).ok();
    assert_eq!(dash.map(|l| l.glyph), Some("md-plus_circle".into()));
}

#[test]
fn refuses_other_shapes() {
    for bad in [
        r#"export const ICON_X = "\u{F03E2}";"#,
        r#"export const ICON_X = "\u{F03E2}"; // paperclip"#,
        r#"export const ICON_X = "\uF03E2"; // nf-md-paperclip"#,
        r"export const ICON_X = ICON_Y; // nf-md-paperclip",
    ] {
        assert!(parse_icon_line(bad).is_err(), "{bad}");
    }
}

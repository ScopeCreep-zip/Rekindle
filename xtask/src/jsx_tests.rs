use super::*;

#[test]
fn generics_and_comparisons_are_not_tags() {
    let src = "const a = b < c; const [x] = createSignal<Stage>(\"intro\"); type T = Record<string, number>;";
    assert!(elements(src).is_empty());
    let text = "<p>Key fingerprint <code>ab</code></p>";
    assert_eq!(elements(text).len(), 2);
    let nested = "<div class=\"a\"><div>inner</div>tail</div>";
    let els = elements(nested);
    assert_eq!(els[0].children.as_deref(), Some("<div>inner</div>tail"));
}

#[test]
fn classes_come_from_static_and_expression_values() {
    let src = r#"<p class="sec-review-fact x"><b class={on() ? "sec-a" : `sec-b`}>k</b><i class={cls()} /></p>"#;
    let els = elements(src);
    assert_eq!(els[0].classes(), vec!["sec-review-fact", "x"]);
    assert_eq!(els[1].classes(), vec!["sec-a", "sec-b"]);
    assert!(els[2].classes().is_empty());
    assert!(els[0].class_is_static() && !els[1].class_is_static());
    assert!(els[1].class_has_template() && !els[0].class_has_template());
}

#[test]
fn class_list_keys_and_spreads_are_read() {
    let src = r#"<b classList={{ "sec-review": on(), active: x, [k]: y }} {...rest} />"#;
    let el = &elements(src)[0];
    assert_eq!(el.classes(), vec!["sec-review", "active", "k"]);
    assert!(el.spread);
    assert_eq!(el.open, 0..src.len());
}

#[test]
fn comments_are_blanked_but_urls_and_globs_are_kept() {
    let src = "a // \u{2192} sec-x\n/* sec-y */ \"rekindle://pair\" accept=\"image/*\" {/* z */}";
    let out = strip_comments(src);
    assert_eq!(out.len(), src.len());
    assert!(!out.contains("sec-x") && !out.contains("sec-y") && !out.contains('z'));
    assert!(out.contains("rekindle://pair") && out.contains("image/*"));
    assert_eq!(out.lines().count(), 2);
}

#[test]
fn visible_text_drops_hidden_icons_and_tags() {
    let src = r#"<button><span class="nf-icon" aria-hidden="true">{ICON_TRASH}</span></button>"#;
    assert_eq!(elements(src)[0].visible_text(), "");
    let src =
        r#"<button><span aria-hidden="true">{ICON_TRASH}</span> <b>Delete</b> {name()}</button>"#;
    assert_eq!(elements(src)[0].visible_text(), "Delete {name()}");
    assert_eq!(elements("<i />")[0].visible_text(), "");
    let src = "<p>a<span>b</span>c</p>";
    assert_eq!(elements(src)[0].span, 0..src.len());
}

use super::*;

/// Combat's `[InfoViewSwitchIn]` runs both of its `Action.click` lines
/// (`combat.ini:803-806`): a repeated trigger key is kept, every line.
#[test]
fn a_repeated_trigger_key_keeps_every_line() {
    let ini = parse(
        "c.ini",
        b"[S]\nAction.click=GroupMessage(1)\nAction.click=LaunchSkin\nGraphics.Trigger=A\nGraphics.Trigger=B\nK=1\nK=2\n",
    )
    .unwrap();
    let kept: Vec<_> = ini.sections[0]
        .entries
        .iter()
        .map(|e| (e.key.as_str(), e.value.as_str()))
        .collect();
    assert_eq!(
        kept,
        [
            ("Action.click", "GroupMessage(1)"),
            ("Action.click", "LaunchSkin"),
            ("Graphics.Trigger", "A"),
            ("Graphics.Trigger", "B"),
            ("K", "1"),
        ]
    );
    assert_eq!(ini.repeated, ["[S] K (line 7)"]);
}

#[test]
fn reads_sections_and_paths() {
    let ini = parse(
        "a.ini",
        b"; c\r\n[Top]\r\nGraphics.top=mainskin\\top.gif\r\nX = 1\r\n\r\n[B]\nk=ALT(\"m\")\n",
    )
    .unwrap();
    let top = ini.section("top").unwrap();
    assert_eq!(top.get("graphics.TOP"), Some("mainskin\\top.gif"));
    assert_eq!(top.get("X"), Some("1"));
    assert_eq!(ini.section("B").unwrap().entries[0].line, 7);
    let ini = parse("a.ini", b"[A]\nk=1\nK=2\n").unwrap();
    assert_eq!(ini.section("A").unwrap().get("k"), Some("1"));
    assert_eq!(ini.repeated, ["[A] K (line 3)"]);
}

/// Wine `PROFILE_Find`: the first section of a name is the one read.
#[test]
fn a_repeated_section_reads_the_first() {
    let ini = parse("a.ini", b"[R]\nMinWidth=10\n[x]\n[r]\nMinWidth=60\nNew=1\n").unwrap();
    assert_eq!(ini.sections.len(), 2);
    let r = ini.section("R").unwrap();
    assert_eq!(r.get("MinWidth"), Some("10"));
    assert_eq!(r.get("New"), None);
    assert_eq!(
        ini.repeated,
        ["[r] (line 4): a repeated section, its keys not read"]
    );
}

/// Wine `PROFILE_Load` stores a line without `=` with a `NULL` value,
/// and `GetPrivateProfileStringW` returns the default for it.
#[test]
fn a_line_without_equals_gives_no_value() {
    let ini = parse("v.ini", b"[Version]\r\nVersion 1.0\r\nLongName=XGlow\r\n").unwrap();
    let v = ini.section("Version").unwrap();
    assert_eq!(v.get("Version"), None);
    assert_eq!(v.get("Version 1.0"), None);
    assert_eq!(v.get("LongName"), Some("XGlow"));
    assert_eq!(ini.repeated, ["[Version] (line 2): no '=', no value"]);
}

/// The key is the text before the first `=`, trimmed, inner spaces kept.
#[test]
fn a_key_may_hold_spaces() {
    let ini = parse("v.ini", b"[Version]\nDesign by -=X-Redeem-DC=-\n").unwrap();
    let v = ini.section("Version").unwrap();
    assert_eq!(v.get("design by -"), Some("X-Redeem-DC=-"));
}

#[test]
fn malformed_and_oversize_are_refused() {
    for bad in [
        &b"k=v\n"[..],
        b"[A]\n=v\n",
        b"[A]\nk]=v\n",
        b"not a pair\n",
        b"[A\n",
        b"[A]\n[a]\nk=\\\n",
        b"[A]\nk=a\\\n",
        b"[A]\nk=\\\"x\n",
        b"[A]\nk=a\\\\b\n",
        b"[A]\nk=\x01\n",
        b"[A]\nk=\xff\n",
    ] {
        assert!(parse("a.ini", bad).is_err(), "{bad:?}");
    }
    let long = format!("[A]\nk={}\n", "x".repeat(MAX_LINE));
    assert!(parse("a.ini", long.as_bytes()).is_err());
    let many = "[A]\n".to_owned() + &";\n".repeat(MAX_LINES);
    assert!(parse("a.ini", many.as_bytes()).is_err());
    assert!(parse("a.ini", &vec![b';'; MAX_INI_BYTES + 1]).is_err());
}

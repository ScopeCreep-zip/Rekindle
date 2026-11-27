use super::qr_text;

#[test]
fn a_qr_is_drawn_light_around_dark() {
    let text = qr_text(&["10".into(), "01".into()]);
    let lines: Vec<&str> = text.lines().collect();
    // (2 rows + 8 quiet) / 2 lines, (2 + 8) columns each.
    assert_eq!(lines.len(), 5);
    assert!(lines.iter().all(|l| l.chars().count() == 10));
    assert!(lines[0].chars().all(|c| c == '\u{2588}'));
    assert_eq!(lines[2].chars().nth(4), Some('\u{2584}'));
    assert_eq!(lines[2].chars().nth(5), Some('\u{2580}'));
}

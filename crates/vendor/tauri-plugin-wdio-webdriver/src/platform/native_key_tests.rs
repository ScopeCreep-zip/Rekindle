use super::NativeKey;

#[test]
fn action_aliases_preserve_navigation_and_shift() {
    for (encoded, key) in [
        ("\u{E004}", NativeKey::Tab),
        ("\u{E006}", NativeKey::NumpadEnter),
        ("\u{E007}", NativeKey::Enter),
        ("\u{E008}", NativeKey::ShiftLeft),
        ("\u{E050}", NativeKey::ShiftRight),
        ("\u{E00C}", NativeKey::Escape),
        ("\u{E00D}", NativeKey::Space),
        (" ", NativeKey::Space),
        ("\u{E012}", NativeKey::Left),
        ("\u{E013}", NativeKey::Up),
        ("\u{E014}", NativeKey::Right),
        ("\u{E015}", NativeKey::Down),
    ] {
        assert_eq!(NativeKey::from_action(encoded), Some(key));
    }
}

#[test]
fn ordinary_text_is_not_a_native_navigation_action() {
    assert_eq!(NativeKey::from_action("a"), None);
    assert_eq!(NativeKey::from_action("Enter Tab"), None);
}

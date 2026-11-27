//! GTK key translation: named keys keep their name, shifted text its shifted keyval.

use gtk::gdk::{keys::constants, ModifierType};

use super::webkit_keyval;

#[test]
fn gtk_translated_keys_preserve_named_keys_and_shifted_text() {
    for (base, state, translated, expected) in [
        (
            constants::Tab,
            ModifierType::empty(),
            constants::Tab,
            constants::Tab,
        ),
        (
            constants::Tab,
            ModifierType::SHIFT_MASK,
            constants::ISO_Left_Tab,
            constants::Tab,
        ),
        (
            constants::a,
            ModifierType::SHIFT_MASK,
            constants::A,
            constants::A,
        ),
    ] {
        assert_eq!(
            webkit_keyval(base, *translated),
            *expected,
            "{base:?} with {state:?}"
        );
    }
}

use super::*;
use rekindle_types::dto::skin::TokenValue;

fn colour(r: u8, g: u8, b: u8, a: u8) -> TokenValue {
    TokenValue::Color { r, g, b, a }
}

fn role(colours: &[SkinPaletteColour], role: &str) -> [u8; 3] {
    colours
        .iter()
        .find(|c| c.role == role)
        .map_or_else(|| panic!("no {role}"), |c| c.rgb)
}

#[test]
fn a_skins_colours_map_to_the_tui_roles() {
    let skin = BTreeMap::from([
        ("accent.text".to_owned(), colour(0x11, 0x22, 0x33, 255)),
        ("bg.dark".to_owned(), colour(0x20, 0x40, 0x60, 255)),
        ("bg.tertiary".to_owned(), colour(255, 255, 255, 128)),
    ]);
    let colours = tui_colours(&resolve(ENTRIES, Base::Dark, &skin));
    assert_eq!(colours.len(), TUI_TOKENS.len());
    assert_eq!(role(&colours, "accent.primary"), [0x11, 0x22, 0x33]);
    assert_eq!(role(&colours, "mode.active"), [0x11, 0x22, 0x33]);
    assert_eq!(role(&colours, "bg.base"), [0x20, 0x40, 0x60]);
    // White at 128/255 over the window background.
    assert_eq!(role(&colours, "bg.highlight"), [0x90, 0xa0, 0xb0]);
    let defaults = tui_colours(&defaults(Base::Dark));
    assert_eq!(role(&defaults, "text.primary"), [0xe7, 0xe7, 0xe7]);
    assert_eq!(role(&colours, "text.primary"), [0xe7, 0xe7, 0xe7]);
}

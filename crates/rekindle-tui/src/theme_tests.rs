use super::*;

#[test]
fn the_registry_palettes_load_with_their_values() {
    let dark = load_theme("rekindle-dark").expect("dark palette loads");
    let light = load_theme("rekindle-light").expect("light palette loads");
    assert!(!dark.is_light());
    assert!(light.is_light());
    assert_eq!(
        Color::from(dark.color("text.primary")),
        Color::Rgb(0xe7, 0xe7, 0xe7)
    );
    assert_eq!(
        Color::from(light.color("text.primary")),
        Color::Rgb(0x1a, 0x1a, 0x1a)
    );
}

/// Every TUI role of the dark registry palette, at `rgb`, but
/// `accent.primary` at `accent`.
fn palette(rgb: [u8; 3], accent: [u8; 3]) -> Vec<SkinPaletteColour> {
    let roles: Vec<String> = registry_toml(SkinBase::Dark)
        .split_once("\n[tokens]\n")
        .and_then(|(_, r)| r.split_once("\n[styles]\n"))
        .map(|(r, _)| {
            r.lines()
                .filter_map(|l| l.split('"').nth(1))
                .map(str::to_owned)
                .collect()
        })
        .expect("the registry palette has roles");
    roles
        .into_iter()
        .map(|role| SkinPaletteColour {
            rgb: if role == "accent.primary" {
                accent
            } else {
                rgb
            },
            role,
        })
        .collect()
}

fn skin(base: SkinBase, palette: Vec<SkinPaletteColour>) -> SkinApplied {
    SkinApplied {
        id: "pkg:0123".into(),
        base,
        tokens_href: String::new(),
        regions: Vec::new(),
        images: Vec::new(),
        palette,
        shells: Vec::new(),
        title_logo: None,
    }
}

#[test]
fn a_skin_palette_becomes_the_tui_theme_colours() {
    let mut t = ThemeManager::load("rekindle-dark").expect("dark loads");
    t.depth = ColourDepth::TrueColour;
    let before = t.color("accent.primary");
    assert_eq!(before, Color::Rgb(0x34, 0x91, 0xd8));
    t.apply_skin(&skin(
        SkinBase::Light,
        palette([0x10, 0x20, 0x30], [0xaa, 0xbb, 0xcc]),
    ))
    .expect("the palette applies");
    assert_eq!(t.color("accent.primary"), Color::Rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(t.color("text.primary"), Color::Rgb(0x10, 0x20, 0x30));
    // The styles resolve from the skin's colours, and its base is kept.
    assert_eq!(t.focused_border().fg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
    assert_eq!(t.style("keyword").fg, Some(Color::Rgb(0xaa, 0xbb, 0xcc)));
    assert!(t.is_light());
    assert_eq!(t.name(), "pkg:0123");
    t.depth = ColourDepth::Ansi256;
    assert_eq!(t.color("accent.primary"), Color::Indexed(146));
}

#[test]
fn a_skin_changed_event_swaps_the_theme() {
    use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};
    let mut t = ThemeManager::load("rekindle-dark").expect("dark loads");
    t.depth = ColourDepth::TrueColour;
    let other = SubscriptionEvent::System(SystemEvent::AuditChainBroken { cursor: 1 });
    assert!(t.on_event(&other).is_none());
    let changed = SubscriptionEvent::System(SystemEvent::SkinChanged {
        skin: skin(SkinBase::Dark, palette([9, 9, 9], [0x51, 0xe3, 0x85])),
    });
    assert!(matches!(t.on_event(&changed), Some(Ok(()))));
    assert_eq!(t.color("accent.primary"), Color::Rgb(0x51, 0xe3, 0x85));
    assert_eq!(t.unfocused_border().fg, Some(Color::Rgb(9, 9, 9)));
}

#[test]
fn a_palette_with_a_stray_or_missing_role_is_refused_and_the_theme_kept() {
    let mut t = ThemeManager::load("rekindle-dark").expect("dark loads");
    let mut stray = palette([1, 2, 3], [4, 5, 6]);
    stray[0].role = "x\" = \"#000000".into();
    assert!(t.apply_skin(&skin(SkinBase::Dark, stray)).is_err());
    let mut short = palette([1, 2, 3], [4, 5, 6]);
    short.pop();
    assert!(t.apply_skin(&skin(SkinBase::Dark, short)).is_err());
    assert_eq!(t.name(), "rekindle-dark");
    assert!(!t.is_light());
}

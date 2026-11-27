use super::*;

#[test]
fn only_an_advertised_direct_colour_terminal_gets_rgb() {
    assert_eq!(ColourDepth::of(Some("truecolor")), ColourDepth::TrueColour);
    assert_eq!(ColourDepth::of(Some("24bit")), ColourDepth::TrueColour);
    assert_eq!(ColourDepth::of(None), ColourDepth::Ansi256);
    assert_eq!(ColourDepth::of(Some("yes")), ColourDepth::Ansi256);
    let c = Color::Rgb(0x34, 0x91, 0xd8);
    assert_eq!(ColourDepth::TrueColour.fit(c), c);
}

#[test]
fn rgb_maps_to_the_nearest_xterm_index() {
    assert_eq!(nearest_256([0, 0, 0]), 16);
    assert_eq!(nearest_256([255, 255, 255]), 231);
    assert_eq!(nearest_256([255, 0, 0]), 196);
    // #3491d8 → cube (1, 2, 4) = 16 + 36 + 12 + 4.
    assert_eq!(nearest_256([0x34, 0x91, 0xd8]), 68);
    // A mid grey lies on the ramp: 8 + 10 × 11 = 118.
    assert_eq!(nearest_256([118, 118, 118]), 243);
    assert_eq!(
        ColourDepth::Ansi256.fit(Color::Rgb(255, 0, 0)),
        Color::Indexed(196)
    );
}

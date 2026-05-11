//! Color palette system — catppuccin themes, gradients, degradation, semantic mapping.

pub mod catppuccin;
pub mod degradation;
pub mod gradient;
pub mod semantic;

pub use degradation::ColorTier;
pub use gradient::Gradient;
pub use semantic::SemanticPalette;

/// Raw palette — 14 accent colors + 6 surface colors from catppuccin.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    // Accents
    pub rosewater: (u8, u8, u8),
    pub flamingo: (u8, u8, u8),
    pub pink: (u8, u8, u8),
    pub mauve: (u8, u8, u8),
    pub red: (u8, u8, u8),
    pub maroon: (u8, u8, u8),
    pub peach: (u8, u8, u8),
    pub yellow: (u8, u8, u8),
    pub green: (u8, u8, u8),
    pub teal: (u8, u8, u8),
    pub sky: (u8, u8, u8),
    pub sapphire: (u8, u8, u8),
    pub blue: (u8, u8, u8),
    pub lavender: (u8, u8, u8),
    // Surfaces
    pub text: (u8, u8, u8),
    pub subtext1: (u8, u8, u8),
    pub subtext0: (u8, u8, u8),
    pub overlay2: (u8, u8, u8),
    pub overlay1: (u8, u8, u8),
    pub overlay0: (u8, u8, u8),
    pub surface2: (u8, u8, u8),
    pub surface1: (u8, u8, u8),
    pub surface0: (u8, u8, u8),
    pub base: (u8, u8, u8),
    pub mantle: (u8, u8, u8),
    pub crust: (u8, u8, u8),
}

impl Palette {
    /// Whether this palette is light (latte) or dark (frappe/macchiato/mocha).
    pub fn is_light(&self) -> bool {
        // Light themes have luminance > 128 on the base color
        let (r, g, b) = self.base;
        (u16::from(r) + u16::from(g) + u16::from(b)) / 3 > 128
    }
}

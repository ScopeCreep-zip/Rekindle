//! The bundled font allow-list: the only names a `fontFamily` token may
//! carry. A skin names a face; the CSS emitted is the app-owned stack of
//! that face, never the skin's text.

/// One allowed face and the stack the app emits for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontFace {
    /// The name a token file uses.
    pub name: &'static str,
    /// The CSS `font-family` value, app-owned.
    pub stack: &'static str,
}

/// The allow-list. The classic Xfire face first; then faces every desktop
/// the app ships on carries or that the app bundles
/// (`src/assets/fonts/`).
pub const FONTS: &[FontFace] = &[
    FontFace {
        name: "Segoe UI",
        stack: "\"Segoe UI\", \"Tahoma\", sans-serif",
    },
    FontFace {
        name: "Tahoma",
        stack: "\"Tahoma\", \"Segoe UI\", sans-serif",
    },
    FontFace {
        name: "Verdana",
        stack: "\"Verdana\", \"Tahoma\", sans-serif",
    },
    FontFace {
        name: "Symbols Nerd Font Mono",
        stack: "\"Symbols Nerd Font Mono\"",
    },
];

/// The allowed face named `name`.
#[must_use]
pub fn font(name: &str) -> Option<&'static FontFace> {
    FONTS.iter().find(|f| f.name == name)
}

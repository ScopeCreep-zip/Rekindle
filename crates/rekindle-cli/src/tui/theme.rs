//! Theme integration via opaline.
//!
//! Wraps [`opaline::Theme`] with rekindle-specific token extensions
//! and glyph fallback for accessibility.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Theme manager wrapping opaline with app-specific extensions.
pub struct ThemeManager {
    theme: opaline::Theme,
    name: String,
    use_unicode: bool,
}

impl ThemeManager {
    /// Load a theme by name from opaline builtins or user themes.
    pub fn load(name: &str) -> anyhow::Result<Self> {
        let theme = opaline::load_by_name(name)
            .ok_or_else(|| anyhow::anyhow!("theme '{name}' not found"))?;
        let use_unicode = crate::output::color::ColorSupport::use_unicode();
        Ok(Self {
            theme,
            name: name.to_string(),
            use_unicode,
        })
    }

    /// Semantic color accessor — returns ratatui Color.
    pub fn color(&self, token: &str) -> Color {
        self.theme.color(token).into()
    }

    /// Semantic style accessor — returns ratatui Style.
    pub fn style(&self, name: &str) -> Style {
        self.theme.style(name).into()
    }

    /// Create a styled span.
    pub fn span<'a>(&self, style_name: &str, content: &'a str) -> Span<'a> {
        self.theme.span(style_name, content)
    }

    /// Whether this is a light theme.
    pub fn is_light(&self) -> bool {
        self.theme.is_light()
    }

    /// Theme name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether Unicode glyphs are available.
    pub fn use_unicode(&self) -> bool {
        self.use_unicode
    }

    /// Status glyph — uses text labels for accessibility.
    pub fn status_glyph(&self, pass: bool) -> &'static str {
        if pass {
            if self.use_unicode {
                "●"
            } else {
                "[OK]"
            }
        } else if self.use_unicode {
            "○"
        } else {
            "[--]"
        }
    }

    /// Focused border style.
    pub fn focused_border(&self) -> Style {
        Style::default().fg(self.color("accent.primary"))
    }

    /// Unfocused border style.
    pub fn unfocused_border(&self) -> Style {
        Style::default().fg(self.color("border.unfocused"))
    }
}

//! Theme manager — loads palette by name, constructs semantic palette,
//! exposes all visual primitives for the TUI render layer.
//!
//! Replaces the thin opaline wrapper with a full color/gradient system.
//! Every color decision in the TUI goes through ThemeManager. No raw
//! RGB values appear in any render code outside this module.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use super::palette::{catppuccin, ColorTier, Gradient, SemanticPalette, Palette};

use crate::v2::output::color::ColorSupport;

/// Theme manager — the single source of all visual primitives.
///
/// Constructed once at TUI startup. Immutable thereafter. Every
/// component and view borrows this for rendering.
pub struct ThemeManager {
    /// The semantic palette (all colors pre-degraded to detected tier).
    pub palette: SemanticPalette,
    /// Raw palette for direct RGB access (gradient construction, custom widgets).
    raw: Palette,
    /// Theme name for display in help overlay.
    name: String,
    /// Detected color tier.
    tier: ColorTier,
    /// Whether Unicode glyphs are available.
    use_unicode: bool,
}

impl ThemeManager {
    /// Load a theme by name. Falls back to catppuccin-frappe if not found.
    pub fn load(name: &str) -> Self {
        let raw = catppuccin::by_name(name).unwrap_or_else(|| {
            tracing::warn!(theme = name, "theme not found, using catppuccin-frappe");
            catppuccin::FRAPPE
        });
        let tier = ColorTier::detect();
        let palette = SemanticPalette::from_palette(&raw, tier);
        let use_unicode = ColorSupport::use_unicode();

        tracing::info!(
            theme = name,
            tier = ?tier,
            unicode = use_unicode,
            light = raw.is_light(),
            "theme loaded"
        );

        Self {
            palette,
            raw,
            name: name.to_string(),
            tier,
            use_unicode,
        }
    }

    // ── Accessors ────────────────────────────────────────────

    /// Theme name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this is a light theme.
    pub fn is_light(&self) -> bool {
        self.raw.is_light()
    }

    /// Whether Unicode glyphs are available.
    pub fn use_unicode(&self) -> bool {
        self.use_unicode
    }

    /// Detected color tier.
    pub fn tier(&self) -> ColorTier {
        self.tier
    }

    /// Available theme names.
    pub fn available_themes() -> &'static [&'static str] {
        catppuccin::available_themes()
    }

    // ── Color accessors ──────────────────────────────────────

    /// Get a semantic color by token name.
    pub fn color(&self, token: &str) -> Color {
        match token {
            "border" => self.palette.border,
            "border.focused" => self.palette.border_focused,
            "border.dim" => self.palette.border_dim,
            "title" => self.palette.title,
            "divider" => self.palette.divider,
            "text.primary" => self.palette.text_primary,
            "text.secondary" => self.palette.text_secondary,
            "text.dim" => self.palette.text_dim,
            "text.muted" => self.palette.text_muted,
            "selected.bg" => self.palette.selected_bg,
            "selected.fg" => self.palette.selected_fg,
            "accent.primary" => self.palette.accent_primary,
            "accent.secondary" => self.palette.accent_secondary,
            "success" => self.palette.success,
            "warning" => self.palette.warning,
            "error" => self.palette.error,
            "info" => self.palette.info,
            "bg.base" => self.palette.base_bg,
            "bg.mantle" => self.palette.mantle_bg,
            "bg.crust" => self.palette.crust_bg,
            "bg.surface" => self.palette.surface_bg,
            "bg.elevated" => self.palette.surface_elevated,
            "meter.bg" => self.palette.meter_bg,
            "highlight.soft" => self.palette.highlight_soft,
            "highlight.badge" => self.palette.highlight_badge,
            "highlight.mention" => self.palette.highlight_mention,
            "online" => self.palette.online,
            "away" => self.palette.away,
            "busy" => self.palette.busy,
            "offline" => self.palette.offline,
            unknown => {
                tracing::warn!(token = unknown, "unknown theme color token");
                self.palette.text_primary
            }
        }
    }

    // ── Style accessors ──────────────────────────────────────

    /// Get a semantic style by name.
    pub fn style(&self, name: &str) -> Style {
        match name {
            "border" => self.palette.border_style(),
            "border.focused" => self.palette.focused_border_style(),
            "border.dim" => self.palette.dim_border_style(),
            "title" => self.palette.title_style(),
            "secondary" => self.palette.secondary_style(),
            "dim" | "dimmed" | "muted" => self.palette.dim_style(),
            "selected" => self.palette.selected_style(),
            "accent" => self.palette.accent_style(),
            "success" => self.palette.success_style(),
            "warning" => self.palette.warning_style(),
            "error" => self.palette.error_style(),
            "keyword" => Style::new().fg(self.palette.accent_primary),
            _ => self.palette.text_style(),
        }
    }

    /// Create a styled span.
    pub fn span<'a>(&self, style_name: &str, content: &'a str) -> Span<'a> {
        Span::styled(content.to_string(), self.style(style_name))
    }

    // ── Border styles ────────────────────────────────────────

    /// Focused border style.
    pub fn focused_border(&self) -> Style {
        self.palette.focused_border_style()
    }

    /// Unfocused border style.
    pub fn unfocused_border(&self) -> Style {
        self.palette.border_style()
    }

    // ── Gradient accessors ───────────────────────────────────

    /// CPU utilization gradient (teal → sapphire → lavender).
    pub fn gradient_cpu(&self) -> &Gradient {
        &self.palette.gradient_cpu
    }

    /// Temperature gradient (green → yellow → red).
    pub fn gradient_temp(&self) -> &Gradient {
        &self.palette.gradient_temp
    }

    /// Memory used gradient (green → teal → sky).
    pub fn gradient_mem_used(&self) -> &Gradient {
        &self.palette.gradient_mem_used
    }

    /// Memory free gradient (mauve → lavender → blue).
    pub fn gradient_mem_free(&self) -> &Gradient {
        &self.palette.gradient_mem_free
    }

    /// Network download gradient (peach → maroon → red).
    pub fn gradient_net_download(&self) -> &Gradient {
        &self.palette.gradient_net_download
    }

    /// Network upload gradient (green → teal → sky).
    pub fn gradient_net_upload(&self) -> &Gradient {
        &self.palette.gradient_net_upload
    }

    /// Process activity gradient (sapphire → lavender → mauve).
    pub fn gradient_process(&self) -> &Gradient {
        &self.palette.gradient_process
    }

    // ── Status glyphs ────────────────────────────────────────

    /// Status glyph with text label for accessibility.
    pub fn status_glyph(&self, pass: bool) -> &'static str {
        SemanticPalette::status_glyph(pass, self.use_unicode)
    }

    /// Presence indicator: (glyph, text_label, color).
    pub fn presence_indicator(&self, status: &str) -> (&'static str, &'static str, Color) {
        self.palette.presence_indicator(status, self.use_unicode)
    }

    // ── Mode badges ──────────────────────────────────────────

    /// Mode badge style for status bar [NORMAL] indicator.
    pub fn mode_normal_style(&self) -> Style {
        Style::new()
            .fg(self.palette.base_bg)
            .bg(self.palette.accent_primary)
            .add_modifier(Modifier::BOLD)
    }

    /// Mode badge style for status bar [INSERT] indicator.
    pub fn mode_insert_style(&self) -> Style {
        Style::new()
            .fg(self.palette.base_bg)
            .bg(self.palette.success)
            .add_modifier(Modifier::BOLD)
    }

    /// Mode badge style for status bar [SEARCH] indicator.
    pub fn mode_search_style(&self) -> Style {
        Style::new()
            .fg(self.palette.base_bg)
            .bg(self.palette.warning)
            .add_modifier(Modifier::BOLD)
    }

    // ── Custom gradient construction ─────────────────────────

    /// Build a custom two-color gradient from raw palette colors.
    pub fn custom_gradient_two(&self, start_token: &str, end_token: &str) -> Gradient {
        let start = self.raw_rgb(start_token);
        let end = self.raw_rgb(end_token);
        if self.tier.has_true_color() {
            Gradient::two_color(start, end)
        } else {
            Gradient::tty_two_step(
                super::palette::degradation::degrade(start, self.tier),
                super::palette::degradation::degrade(end, self.tier),
            )
        }
    }

    /// Build a custom three-color gradient from raw palette colors.
    pub fn custom_gradient_three(&self, start_token: &str, mid_token: &str, end_token: &str) -> Gradient {
        let start = self.raw_rgb(start_token);
        let mid = self.raw_rgb(mid_token);
        let end = self.raw_rgb(end_token);
        if self.tier.has_true_color() {
            Gradient::three_color(start, mid, end)
        } else {
            Gradient::tty_three_step(
                super::palette::degradation::degrade(start, self.tier),
                super::palette::degradation::degrade(mid, self.tier),
                super::palette::degradation::degrade(end, self.tier),
            )
        }
    }

    /// Get raw RGB tuple from the palette by token name.
    fn raw_rgb(&self, token: &str) -> (u8, u8, u8) {
        match token {
            "rosewater" => self.raw.rosewater,
            "flamingo" => self.raw.flamingo,
            "pink" => self.raw.pink,
            "mauve" => self.raw.mauve,
            "red" => self.raw.red,
            "maroon" => self.raw.maroon,
            "peach" => self.raw.peach,
            "yellow" => self.raw.yellow,
            "green" => self.raw.green,
            "teal" => self.raw.teal,
            "sky" => self.raw.sky,
            "sapphire" => self.raw.sapphire,
            "blue" => self.raw.blue,
            "lavender" => self.raw.lavender,
            "base" => self.raw.base,
            "surface0" => self.raw.surface0,
            "surface1" => self.raw.surface1,
            _ => self.raw.text,
        }
    }
}

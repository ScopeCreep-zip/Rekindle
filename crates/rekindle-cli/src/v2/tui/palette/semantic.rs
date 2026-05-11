//! Semantic color palette — maps palette colors to UI roles.
//!
//! Every color used in the TUI is accessed through this struct.
//! No raw RGB values appear in render code. All colors are pre-degraded
//! to the detected color tier at construction time.

use ratatui::style::{Color, Modifier, Style};

use super::degradation::{degrade, ColorTier};
use super::gradient::Gradient;
use super::Palette;

/// Complete semantic palette — all colors and gradients for the TUI.
#[derive(Clone)]
pub struct SemanticPalette {
    // ── Panel structure ──────────────────────────────────────
    pub border: Color,
    pub border_focused: Color,
    pub border_dim: Color,
    pub title: Color,
    pub divider: Color,

    // ── Text hierarchy ───────────────────────────────────────
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    // ── Interactive states ────────────────────────────────────
    pub selected_bg: Color,
    pub selected_fg: Color,
    pub accent_primary: Color,
    pub accent_secondary: Color,

    // ── Semantic colors ──────────────────────────────────────
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,

    // ── Background ───────────────────────────────────────────
    pub base_bg: Color,
    pub mantle_bg: Color,
    /// Deepest background — nested panel insets, modal backdrops.
    pub crust_bg: Color,
    pub surface_bg: Color,
    /// Elevated surface — hover states, active card backgrounds.
    pub surface_elevated: Color,
    pub meter_bg: Color,

    // ── Presence indicators ──────────────────────────────────
    pub online: Color,
    pub away: Color,
    pub busy: Color,
    pub offline: Color,

    // ── Data visualization gradients ─────────────────────────
    pub gradient_cpu: Gradient,
    pub gradient_temp: Gradient,
    pub gradient_mem_used: Gradient,
    pub gradient_mem_free: Gradient,
    pub gradient_net_download: Gradient,
    pub gradient_net_upload: Gradient,
    pub gradient_process: Gradient,

    // ── Accent highlights ─────────────────────────────────────
    /// Soft warm highlight — link hover, gentle emphasis.
    pub highlight_soft: Color,
    /// Notification/badge accent — unread counts, alert badges.
    pub highlight_badge: Color,
    /// Mention/ping accent — @mention highlighting in messages.
    pub highlight_mention: Color,

}

impl SemanticPalette {
    /// Construct from a raw palette and detected color tier.
    pub fn from_palette(palette: &Palette, tier: ColorTier) -> Self {
        let d = |rgb: (u8, u8, u8)| degrade(rgb, tier);

        Self {
            // Panel structure
            border: d(palette.surface1),
            border_focused: d(palette.blue),
            border_dim: d(palette.surface0),
            title: d(palette.text),
            divider: d(palette.overlay2),

            // Text hierarchy
            text_primary: d(palette.text),
            text_secondary: d(palette.subtext1),
            text_dim: d(palette.subtext0),
            text_muted: d(palette.overlay1),

            // Interactive states
            selected_bg: d(palette.surface1),
            selected_fg: d(palette.blue),
            accent_primary: d(palette.blue),
            accent_secondary: d(palette.mauve),

            // Semantic colors
            success: d(palette.green),
            warning: d(palette.yellow),
            error: d(palette.red),
            info: d(palette.sapphire),

            // Background
            base_bg: d(palette.base),
            mantle_bg: d(palette.mantle),
            crust_bg: d(palette.crust),
            surface_bg: d(palette.surface0),
            surface_elevated: d(palette.surface2),
            meter_bg: d(palette.surface1),

            // Presence
            online: d(palette.green),
            away: d(palette.yellow),
            busy: d(palette.red),
            offline: d(palette.overlay0),

            // Gradients — three-color transitions using palette accents
            gradient_cpu: if tier.has_true_color() {
                Gradient::three_color(palette.teal, palette.sapphire, palette.lavender)
            } else {
                Gradient::tty_three_step(Color::Cyan, Color::Blue, Color::Magenta)
            },
            gradient_temp: if tier.has_true_color() {
                Gradient::three_color(palette.green, palette.yellow, palette.red)
            } else {
                Gradient::tty_three_step(Color::Green, Color::Yellow, Color::Red)
            },
            gradient_mem_used: if tier.has_true_color() {
                Gradient::three_color(palette.green, palette.teal, palette.sky)
            } else {
                Gradient::tty_two_step(Color::Green, Color::Cyan)
            },
            gradient_mem_free: if tier.has_true_color() {
                Gradient::three_color(palette.mauve, palette.lavender, palette.blue)
            } else {
                Gradient::tty_two_step(Color::Magenta, Color::Blue)
            },
            gradient_net_download: if tier.has_true_color() {
                Gradient::three_color(palette.peach, palette.maroon, palette.red)
            } else {
                Gradient::tty_three_step(Color::Yellow, Color::Red, Color::LightRed)
            },
            gradient_net_upload: if tier.has_true_color() {
                Gradient::three_color(palette.green, palette.teal, palette.sky)
            } else {
                Gradient::tty_two_step(Color::Green, Color::Cyan)
            },
            gradient_process: if tier.has_true_color() {
                Gradient::three_color(palette.sapphire, palette.lavender, palette.mauve)
            } else {
                Gradient::tty_three_step(Color::Blue, Color::Magenta, Color::LightMagenta)
            },

            // Accent highlights
            highlight_soft: d(palette.rosewater),
            highlight_badge: d(palette.flamingo),
            highlight_mention: d(palette.pink),

        }
    }

    // ── Style constructors ───────────────────────────────────

    /// Style for panel borders (unfocused).
    pub fn border_style(&self) -> Style {
        Style::new().fg(self.border)
    }

    /// Style for focused panel borders.
    pub fn focused_border_style(&self) -> Style {
        Style::new().fg(self.border_focused)
    }

    /// Style for dim/inactive borders.
    pub fn dim_border_style(&self) -> Style {
        Style::new().fg(self.border_dim)
    }

    /// Style for panel titles.
    pub fn title_style(&self) -> Style {
        Style::new().fg(self.title).add_modifier(Modifier::BOLD)
    }

    /// Style for primary text.
    pub fn text_style(&self) -> Style {
        Style::new().fg(self.text_primary)
    }

    /// Style for secondary text.
    pub fn secondary_style(&self) -> Style {
        Style::new().fg(self.text_secondary)
    }

    /// Style for dim/muted text.
    pub fn dim_style(&self) -> Style {
        Style::new().fg(self.text_dim)
    }

    /// Style for selected/highlighted items.
    pub fn selected_style(&self) -> Style {
        Style::new().fg(self.selected_fg).bg(self.selected_bg)
    }

    /// Style for bold accent text.
    pub fn accent_style(&self) -> Style {
        Style::new().fg(self.accent_primary).add_modifier(Modifier::BOLD)
    }

    /// Style for success messages.
    pub fn success_style(&self) -> Style {
        Style::new().fg(self.success)
    }

    /// Style for warning messages.
    pub fn warning_style(&self) -> Style {
        Style::new().fg(self.warning)
    }

    /// Style for error messages.
    pub fn error_style(&self) -> Style {
        Style::new().fg(self.error).add_modifier(Modifier::BOLD)
    }

    /// Status glyph — text label for accessibility, glyph for visual.
    pub fn status_glyph(pass: bool, use_unicode: bool) -> &'static str {
        if pass {
            if use_unicode { "●" } else { "[OK]" }
        } else if use_unicode { "○" } else { "[--]" }
    }

    /// Presence glyph + text label. Never color alone.
    pub fn presence_indicator(&self, status: &str, use_unicode: bool) -> (&'static str, &'static str, Color) {
        match status {
            "online" => (
                if use_unicode { "●" } else { "o" },
                "[ONLINE]",
                self.online,
            ),
            "away" => (
                if use_unicode { "◐" } else { "~" },
                "[AWAY]",
                self.away,
            ),
            "busy" => (
                if use_unicode { "●" } else { "-" },
                "[BUSY]",
                self.busy,
            ),
            "offline" => (
                if use_unicode { "○" } else { "." },
                "[OFFLINE]",
                self.offline,
            ),
            _ => (
                if use_unicode { "◌" } else { "?" },
                "[?]",
                self.text_muted,
            ),
        }
    }
}

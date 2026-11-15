//! The composited pairs step 6 measures beyond the registry's own
//! constraints: labels on `color-mix(in srgb, <fill> <share>, transparent)`
//! tints (d3 §6.3 D4), the danger fill as a graphic (D5) and the status
//! colours on the StatusDot cutout and the rows a dot is drawn on (S2, S3).
//! Each row cites the rule it was read from (`src/styles/theme/*.css`) and
//! the grounds `src/styles/regions.toml` declares for it.

/// The share of a tint, thousandths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TintShare {
    Fixed(u16),
    /// A `Fraction` token of the registry (`tint.fill`).
    Token(&'static str),
}

/// A surface something is drawn on: an opaque ground token, or a tint
/// composited over a surface in gamma-encoded sRGB (d3 §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    Token(&'static str),
    Tint {
        token: &'static str,
        share: TintShare,
        over: &'static Ground,
    },
}

/// What the floor protects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor {
    /// WCAG 1.4.3, 4.5:1.
    Text,
    /// WCAG 1.4.11, 3:1.
    Graphic,
}

impl Floor {
    #[must_use]
    pub fn x100(self) -> u16 {
        match self {
            Self::Text => 450,
            Self::Graphic => 300,
        }
    }
}

/// `fg` on each of `on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pair {
    pub rule: &'static str,
    pub fg: &'static str,
    pub on: &'static [Ground],
    pub floor: Floor,
}

const DARK: Ground = Ground::Token("bg.dark");
const PANEL: Ground = Ground::Token("bg.panel");
const STATUS_BAR: Ground = Ground::Token("bg.status");
const PRIMARY: Ground = Ground::Token("bg.primary");
const TITLE_START: Ground = Ground::Token("titlebar.start");
const TITLE_END: Ground = Ground::Token("titlebar.end");

const fn tint(token: &'static str, share: &'static str, over: &'static Ground) -> Ground {
    Ground::Tint {
        token,
        share: TintShare::Token(share),
        over,
    }
}

/// The rows a StatusDot is drawn on (`regions.toml` `.status-dot-*`):
/// the window, the primary and panel grounds and the selected row (accent
/// 30 % over the window).
pub const STATUS_ROWS: &[Ground] = &[
    DARK,
    PRIMARY,
    PANEL,
    Ground::Tint {
        token: "accent.fill",
        share: TintShare::Fixed(300),
        over: &DARK,
    },
];

/// The five presence colours, in StatusDot order.
pub const STATUSES: [&str; 5] = [
    "status.online",
    "status.away",
    "status.busy",
    "status.ingame",
    "status.offline",
];

pub const PAIRS: &[Pair] = &[
    // `.xfire-titlebar-btn-close:hover` (00-rules.css:129-131).
    Pair {
        rule: "D4",
        fg: "danger.fg",
        on: &[
            tint("danger.fill", "tint.fill-strong", &TITLE_START),
            tint("danger.fill", "tint.fill-strong", &TITLE_END),
        ],
        floor: Floor::Text,
    },
    // `.form-btn-danger` (00-rules.css:229-232), `.voice-btn-active`
    // (03-member-profile-popup.css:6-8).
    Pair {
        rule: "D4",
        fg: "danger.fg",
        on: &[
            tint("danger.fill", "tint.fill", &DARK),
            tint("danger.fill", "tint.fill", &PANEL),
            tint("danger.fill", "tint.fill", &STATUS_BAR),
        ],
        floor: Floor::Text,
    },
    // `.voice-btn-disconnect` (03-member-profile-popup.css:11-13).
    Pair {
        rule: "D4",
        fg: "danger.fg",
        on: &[
            tint("danger.fill", "tint.fill-soft", &DARK),
            tint("danger.fill", "tint.fill-soft", &STATUS_BAR),
        ],
        floor: Floor::Text,
    },
    // `.pending-btn-accept` (04-member-profile-popup.css:375-383),
    // `.channel-voice-join-btn` (07-member-profile-popup.css:397-404).
    Pair {
        rule: "D4",
        fg: "text.on-fill",
        on: &[tint("status.online", "tint.fill", &DARK)],
        floor: Floor::Text,
    },
    // D5: the danger fill as an icon or a boundary on the grounds.
    Pair {
        rule: "D5",
        fg: "danger.fill",
        on: &[DARK, PANEL],
        floor: Floor::Graphic,
    },
];

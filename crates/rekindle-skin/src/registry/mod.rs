//! The closed token registry (plan E7.7.1; Telegram `colors.palette`, VS
//! Code `registerColor`): one Rust table, [`ENTRIES`], every token the app
//! themes or fixes, with its type, its default on each of the four bases,
//! an optional derivation, its contrast constraints and its class.
//!
//! The table is the one source of truth. `cargo xtask gen-skin-registry`
//! generates from it the app's custom-property declarations
//! (`src/styles/generated/tokens.css`: Tailwind 4 `@theme` plus `@property`
//! registrations), the TS types, the TUI palettes and a JSON Schema for
//! external editors; `cargo xtask check` fails when they are stale. A skin
//! may omit a token, which then takes the default of its base (or its
//! derivation, when the skin sets the token it derives from); the defaults
//! themselves pass the constraints in this crate's tests.
//!
//! Security-surface tokens (`--sec-*`) are absent: they live only in
//! `@layer sec` of `global.css` and no skin reaches them (plan E7.7.0).

pub mod dialog;
mod fonts;
mod slots;
mod table;

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::dto::skin::{SkinBase, SkinContrast, TokenValue};

pub use dialog::DIALOG;
pub use fonts::{font, FontFace, FONTS};
pub use slots::{IconSlot, SoundCue};
/// The base rows, with the title grounds at their defaults: what the import
/// walks before any dialog role is derived (ST1 ruling 2).
pub const BASE_TABLE: &[TokenEntry] = &BASE_AND_TITLE;
const BASE_AND_TITLE: [TokenEntry; table::BASE.len() + 2] =
    concat(table::BASE, &dialog::TITLE_AS_BEFORE);

/// Every row: the base table, then the dialog roles.
pub const ENTRIES: &[TokenEntry] = &BASE_THEN_DIALOG;
const BASE_THEN_DIALOG: [TokenEntry; table::BASE.len() + DIALOG.len()] =
    concat(table::BASE, DIALOG);

const fn concat<const N: usize>(a: &[TokenEntry], b: &[TokenEntry]) -> [TokenEntry; N] {
    assert!(a.len() + b.len() == N && !a.is_empty());
    let mut out = [a[0]; N];
    let mut i = 0;
    while i < N {
        out[i] = if i < a.len() { a[i] } else { b[i - a.len()] };
        i += 1;
    }
    out
}

use crate::colour;

/// A token's type: the four of the restricted DTCG profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Color,
    /// Integer CSS pixels within `min..=max`.
    Dimension {
        min: i32,
        max: i32,
    },
    /// Thousandths within `min..=max`.
    Number {
        min: i64,
        max: i64,
    },
    /// A share of the whole in thousandths within `min..=max` (a DTCG
    /// number, `0.6`), emitted as a CSS percentage (`60%`): the strength of
    /// a `color-mix(in srgb, <fill> <share>, transparent)` tint.
    Fraction {
        min: i64,
        max: i64,
    },
    FontFamily,
}

impl TokenType {
    /// The DTCG `$type` string.
    #[must_use]
    pub fn dtcg(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Dimension { .. } => "dimension",
            Self::Number { .. } | Self::Fraction { .. } => "number",
            Self::FontFamily => "fontFamily",
        }
    }
}

/// A default value, as numbers (a font by its allow-list name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultValue {
    /// `0xRRGGBBAA`.
    Color(u32),
    Dimension(i32),
    Number(i64),
    Font(&'static str),
}

impl DefaultValue {
    #[must_use]
    pub fn value(self) -> TokenValue {
        match self {
            Self::Color(packed) => {
                let [r, g, b, a] = packed.to_be_bytes();
                TokenValue::Color { r, g, b, a }
            }
            Self::Dimension(px) => TokenValue::Dimension(px),
            Self::Number(n) => TokenValue::Number(n),
            Self::Font(name) => TokenValue::FontFamily(name.to_owned()),
        }
    }
}

/// The four bases a variant resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Base {
    Dark,
    Light,
    HcDark,
    HcLight,
}

impl Base {
    pub const ALL: [Self; 4] = [Self::Dark, Self::Light, Self::HcDark, Self::HcLight];

    #[must_use]
    pub fn of(base: SkinBase, contrast: SkinContrast) -> Self {
        match (base, contrast) {
            (SkinBase::Dark, SkinContrast::Standard) => Self::Dark,
            (SkinBase::Light, SkinContrast::Standard) => Self::Light,
            (SkinBase::Dark, SkinContrast::High) => Self::HcDark,
            (SkinBase::Light, SkinContrast::High) => Self::HcLight,
        }
    }

    /// The registry's key for the base (`dark`, `light`, `hcDark`, `hcLight`).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::HcDark => "hcDark",
            Self::HcLight => "hcLight",
        }
    }
}

/// The defaults on each base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    pub dark: DefaultValue,
    pub light: DefaultValue,
    pub hc_dark: DefaultValue,
    pub hc_light: DefaultValue,
}

impl Defaults {
    #[must_use]
    pub fn on(&self, base: Base) -> DefaultValue {
        match base {
            Base::Dark => self.dark,
            Base::Light => self.light,
            Base::HcDark => self.hc_dark,
            Base::HcLight => self.hc_light,
        }
    }
}

/// How a token follows another when a skin sets that one and omits this
/// one. Weights and amounts are thousandths; colours mix in gamma-encoded
/// sRGB, as `color-mix(in srgb, …)` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Derivation {
    /// `a` at `a_weight`, `b` at the rest.
    Mix {
        a: &'static str,
        b: &'static str,
        a_weight: u16,
    },
    /// `of` with its alpha scaled to `alpha`.
    Transparent { of: &'static str, alpha: u16 },
    /// `of` mixed toward white by `by`.
    Lighten { of: &'static str, by: u16 },
    /// `of` mixed toward black by `by`.
    Darken { of: &'static str, by: u16 },
}

impl Derivation {
    /// The tokens it reads.
    #[must_use]
    pub fn sources(&self) -> Vec<&'static str> {
        match *self {
            Self::Mix { a, b, .. } => vec![a, b],
            Self::Transparent { of, .. } | Self::Lighten { of, .. } | Self::Darken { of, .. } => {
                vec![of]
            }
        }
    }

    /// The derived value, or `None` when a source is not a resolved colour.
    #[must_use]
    pub fn apply(&self, resolved: &BTreeMap<&str, TokenValue>) -> Option<TokenValue> {
        let get = |id: &str| match resolved.get(id) {
            Some(TokenValue::Color { r, g, b, a }) => Some([*r, *g, *b, *a]),
            _ => None,
        };
        let [r, g, b, a] = match *self {
            Self::Mix { a, b, a_weight } => colour::mix(get(a)?, get(b)?, a_weight),
            Self::Transparent { of, alpha } => {
                let [r, g, b, a] = get(of)?;
                [r, g, b, colour::scale(a, alpha)]
            }
            Self::Lighten { of, by } => {
                let c = get(of)?;
                colour::mix([255, 255, 255, c[3]], c, by)
            }
            Self::Darken { of, by } => {
                let c = get(of)?;
                colour::mix([0, 0, 0, c[3]], c, by)
            }
        };
        Some(TokenValue::Color { r, g, b, a })
    }
}

/// A contrast floor against grounds, in hundredths of the WCAG ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    pub on: &'static [&'static str],
    pub min_ratio_x100: u16,
}

/// Whether a skin may set the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Themeable,
    /// The same on every skin (black scrims, white text on fills, the icon
    /// font); a skin that sets it is refused.
    Fixed,
}

/// One registry row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEntry {
    /// The DTCG path, dot-separated (`bg.panel`).
    pub id: &'static str,
    /// The CSS custom property the app reads.
    pub css: &'static str,
    pub ty: TokenType,
    pub defaults: Defaults,
    pub derivation: Option<Derivation>,
    pub constraints: &'static [Constraint],
    pub class: Class,
}

/// Every constraint `table`'s defaults break on `base`, as
/// `token on ground: ratio < floor`. Text and grounds are composited over
/// the ground (alpha resolved), as the app paints them.
#[must_use]
pub fn constraint_failures(table: &[TokenEntry], base: Base) -> Vec<String> {
    let resolved = resolve(table, base, &BTreeMap::new());
    let colour_of = |id: &str| {
        resolved
            .iter()
            .find(|(e, _)| e.id == id)
            .and_then(|(_, v)| match v {
                TokenValue::Color { r, g, b, a } => Some([*r, *g, *b, *a]),
                _ => None,
            })
    };
    let mut out = Vec::new();
    for (e, _) in &resolved {
        for c in e.constraints {
            for ground in c.on {
                let (Some(fg), Some(bg)) = (colour_of(e.id), colour_of(ground)) else {
                    out.push(format!("{} on {ground}: not a colour pair", e.id));
                    continue;
                };
                let under = colour::over(bg, [0, 0, 0]);
                let ratio = colour::contrast(colour::over(fg, under), under);
                if ratio * 100.0 < f64::from(c.min_ratio_x100) {
                    out.push(format!(
                        "{} on {ground} ({}): {ratio:.2} < {:.2}",
                        e.id,
                        base.key(),
                        f64::from(c.min_ratio_x100) / 100.0
                    ));
                }
            }
        }
    }
    out
}

/// The row of `id` in `table`.
#[must_use]
pub fn entry_in<'a>(table: &'a [TokenEntry], id: &str) -> Option<&'a TokenEntry> {
    table.iter().find(|e| e.id == id)
}

/// Every token of `table` on `base`: the skin's own value, else the
/// derivation when the skin sets a token it reads, else the base default.
/// A dialog role ([`DIALOG`]) also follows its derivation when a token it
/// reads is itself derived from the skin's colours (`menubar.bg` from
/// `bg.dark` from the skin's `bg.primary`), so a package that lacks the
/// role is served it from its own colours (ST1 ruling 4).
/// `skin` holds only ids of `table` (what [`crate::tokens::parse`] returns).
#[must_use]
pub fn resolve<'a>(
    table: &'a [TokenEntry],
    base: Base,
    skin: &BTreeMap<String, TokenValue>,
) -> Vec<(&'a TokenEntry, TokenValue)> {
    let mut resolved: BTreeMap<&str, TokenValue> = BTreeMap::new();
    for e in table.iter().filter(|e| e.derivation.is_none()) {
        let v = skin
            .get(e.id)
            .cloned()
            .unwrap_or_else(|| e.defaults.on(base).value());
        resolved.insert(e.id, v);
    }
    let mut followed: BTreeSet<&str> = BTreeSet::new();
    for e in table {
        let Some(d) = e.derivation else { continue };
        let role = entry_in(DIALOG, e.id).is_some();
        let follows = d
            .sources()
            .iter()
            .any(|s| skin.contains_key(*s) || (role && followed.contains(s)));
        let derived = (follows && !skin.contains_key(e.id))
            .then(|| d.apply(&resolved))
            .flatten();
        if derived.is_some() {
            followed.insert(e.id);
        }
        let v = skin.get(e.id).cloned().or(derived);
        resolved.insert(e.id, v.unwrap_or_else(|| e.defaults.on(base).value()));
    }
    table
        .iter()
        .filter_map(|e| resolved.remove(e.id).map(|v| (e, v)))
        .collect()
}

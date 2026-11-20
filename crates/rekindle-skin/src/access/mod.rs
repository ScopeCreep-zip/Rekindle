//! Validator step 6 (plan E7.7.1): accessibility and anti-spoofing on a
//! variant after defaults merge ([`crate::registry::resolve`]), measured on
//! composited colours (`skins/research/d3-distinctness.md` §6).
//!
//! * Opaque grounds: every token text or a graphic is laid on is opaque
//!   (our windows are transparent, so there is no glass beneath).
//! * Base check: the window ground (`bg.dark`) sits on the side of the WCAG
//!   crossover its declared base names. The crossover is the luminance
//!   with equal contrast against black and white, `√(1.05 · 0.05) − 0.05`
//!   ≈ 0.179; a dark base needs more contrast against white.
//! * The registry's constraints (1.4.3 text 4.5:1, 1.4.11 graphics 3:1)
//!   and [`pairs::PAIRS`] (D4, D5) on the composited colours.
//! * The full pair model of `src/styles/regions.toml` ([`model`]: tints,
//!   opacities, hover and selected grounds, icons, boundaries) on the
//!   variant's own tokens, the same set `cargo xtask check-contrast-pairs`
//!   measures on the defaults.
//! * D1–D3 on the opaque danger and accent fills; S1–S3 on the statuses.
//!
//! Nothing is auto-fixed: each failure is a [`SkinRule`] naming the rule,
//! the pair and the measured and required values.

pub mod apca;
pub mod model;
pub mod pairs;
pub mod science;
mod skin_model;

use std::collections::BTreeMap;

use rekindle_types::dto::skin::TokenValue;
use rekindle_types::dto::skin::{SkinError, SkinRule};

use crate::colour;
use crate::registry::{Base, TokenEntry};
use pairs::{Floor, Ground, TintShare, PAIRS, STATUSES, STATUS_ROWS};
use science::{distance, linear, luminance, oklch, Vision};

/// D1: the danger hue sector and chroma floor (d3 §4.4).
pub const DANGER_HUE: (f64, f64) = (12.0, 36.0);
pub const DANGER_MIN_CHROMA: f64 = 0.06;
/// D2 and S1: ΔE00 floors, normal vision and simulated CVD (d3 §1.3).
pub const MIN_DISTANCE: f64 = 20.0;
pub const MIN_CVD_DISTANCE: f64 = 14.0;

/// The StatusDot cutout ground (`--sec-status-cutout`, a security-layer
/// token no skin reaches: `src/styles/global.css:105, 145, 170`; the
/// `prefers-contrast: more` value is black on both bases).
#[must_use]
pub fn cutout_ground(base: Base) -> [u8; 3] {
    match base {
        Base::Dark => [0x0a; 3],
        Base::Light => [0xff; 3],
        Base::HcDark | Base::HcLight => [0; 3],
    }
}

/// A resolved variant's colours and shares, by registry id.
struct Variant<'a> {
    colours: BTreeMap<&'a str, [u8; 4]>,
    shares: BTreeMap<&'a str, i64>,
}

impl<'a> Variant<'a> {
    fn new(resolved: &'a [(&'a TokenEntry, TokenValue)]) -> Self {
        let mut colours = BTreeMap::new();
        let mut shares = BTreeMap::new();
        for (e, v) in resolved {
            match v {
                TokenValue::Color { r, g, b, a } => {
                    colours.insert(e.id, [*r, *g, *b, *a]);
                }
                TokenValue::Number(n) => {
                    shares.insert(e.id, *n);
                }
                _ => {}
            }
        }
        Self { colours, shares }
    }

    fn colour(&self, id: &str) -> [u8; 4] {
        self.colours.get(id).copied().unwrap_or([0, 0, 0, 255])
    }

    fn rgb(&self, id: &str) -> [u8; 3] {
        let [r, g, b, _] = self.colour(id);
        [r, g, b]
    }

    fn share(&self, share: TintShare) -> u16 {
        match share {
            TintShare::Fixed(t) => t,
            TintShare::Token(id) => {
                u16::try_from(self.shares.get(id).copied().unwrap_or(1000).clamp(0, 1000))
                    .unwrap_or(1000)
            }
        }
    }

    /// The opaque colour of `ground`: a tint at `share` of its own alpha,
    /// composited in gamma-encoded sRGB over the surface beneath.
    fn surface(&self, ground: &Ground) -> [u8; 3] {
        match ground {
            Ground::Token(id) => self.rgb(id),
            Ground::Tint { token, share, over } => {
                let [tint_r, tint_g, tint_b, tint_a] = self.colour(token);
                let weight = (u32::from(self.share(*share)) * u32::from(tint_a) + 127) / 255;
                let [under_r, under_g, under_b] = self.surface(over);
                let [red, green, blue, _] = colour::mix(
                    [tint_r, tint_g, tint_b, 255],
                    [under_r, under_g, under_b, 255],
                    u16::try_from(weight).unwrap_or(1000),
                );
                [red, green, blue]
            }
        }
    }
}

/// How a ground is named in an error: `bg.dark`, or
/// `danger.fill 60% over bg.dark`.
fn describe(v: &Variant<'_>, ground: &Ground) -> String {
    match ground {
        Ground::Token(id) => (*id).to_owned(),
        Ground::Tint { token, share, over } => {
            let t = v.share(*share);
            format!("{token} {}.{}% over {}", t / 10, t % 10, describe(v, over))
        }
    }
}

/// `x` scaled and floored, so a measured value never prints at its floor.
pub(crate) fn scaled(x: f64, by: f64) -> u32 {
    let v = (x * by).floor();
    if v <= 0.0 {
        0
    } else if v >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        // In range and integral after the checks above.
        format!("{v:.0}").parse().unwrap_or(u32::MAX)
    }
}

fn opaque_grounds<'t>(table: &'t [(&'t TokenEntry, TokenValue)]) -> Vec<&'t str> {
    fn under(g: &Ground, out: &mut Vec<&'static str>) {
        match g {
            Ground::Token(id) => out.push(id),
            Ground::Tint { over, .. } => under(over, out),
        }
    }
    let mut out: Vec<&str> = Vec::new();
    for (e, _) in table {
        for c in e.constraints {
            out.extend(c.on.iter().copied());
        }
    }
    let mut fixed = Vec::new();
    for p in PAIRS {
        for g in p.on {
            under(g, &mut fixed);
        }
    }
    for g in STATUS_ROWS {
        under(g, &mut fixed);
    }
    out.extend(fixed);
    out.sort_unstable();
    out.dedup();
    out
}

/// One declared contrast pair, measured on composited colours (the
/// editor's per-pair report, plan E7.7.5).
#[derive(Debug, Clone, PartialEq)]
pub struct Measured {
    pub rule: String,
    pub fg: String,
    /// How the ground is named (see [`describe`]).
    pub ground: String,
    /// The text composited over the ground, and the opaque ground.
    pub fg_rgb: [u8; 3],
    pub ground_rgb: [u8; 3],
    pub ratio: f64,
    pub required_x100: u16,
}

impl Measured {
    #[must_use]
    pub fn passes(&self) -> bool {
        self.ratio * 100.0 >= f64::from(self.required_x100)
    }

    fn refusal(self) -> Option<SkinRule> {
        (!self.passes()).then(|| SkinRule::Contrast {
            measured_x100: scaled(self.ratio, 100.0),
            required_x100: u32::from(self.required_x100),
            rule: self.rule,
            token: self.fg,
            ground: self.ground,
        })
    }
}

fn measure_on(
    rule: &str,
    v: &Variant<'_>,
    fg: &str,
    under: [u8; 3],
    ground: String,
    min_x100: u16,
) -> Measured {
    let fg_rgb = colour::over(v.colour(fg), under);
    Measured {
        rule: rule.to_owned(),
        fg: fg.to_owned(),
        ground,
        fg_rgb,
        ground_rgb: under,
        ratio: colour::contrast(fg_rgb, under),
        required_x100: min_x100,
    }
}

fn measure(rule: &str, v: &Variant<'_>, fg: &str, ground: &Ground, min_x100: u16) -> Measured {
    measure_on(
        rule,
        v,
        fg,
        v.surface(ground),
        describe(v, ground),
        min_x100,
    )
}

fn contrast_rule(
    rule: &str,
    v: &Variant<'_>,
    fg: &str,
    ground: &Ground,
    min_x100: u16,
) -> Option<SkinRule> {
    measure(rule, v, fg, ground, min_x100).refusal()
}

fn constraint_rule(min_ratio_x100: u16) -> &'static str {
    if min_ratio_x100 >= 450 {
        "1.4.3"
    } else {
        "1.4.11"
    }
}

fn cutout_pair(base: Base, v: &Variant<'_>, x: &str) -> Measured {
    let ground = "the StatusDot cutout".to_owned();
    measure_on("S2", v, x, cutout_ground(base), ground, 300)
}

/// Every declared contrast pair of a variant on `base`, passing or not:
/// the registry's constraints, [`PAIRS`], the pair model, S2 and S3, in
/// that order.
#[must_use]
pub fn measured(base: Base, resolved: &[(&TokenEntry, TokenValue)]) -> Vec<Measured> {
    let v = Variant::new(resolved);
    let mut out = Vec::new();
    for (e, _) in resolved {
        for c in e.constraints {
            let rule = constraint_rule(c.min_ratio_x100);
            for ground in c.on {
                out.push(measure(
                    rule,
                    &v,
                    e.id,
                    &Ground::Token(ground),
                    c.min_ratio_x100,
                ));
            }
        }
    }
    for p in PAIRS {
        for g in p.on {
            out.push(measure(p.rule, &v, p.fg, g, p.floor.x100()));
        }
    }
    out.extend(skin_model::model_pairs(resolved).into_iter().flatten());
    for x in STATUSES.iter().filter(|x| **x != "status.offline") {
        out.push(cutout_pair(base, &v, x));
    }
    for x in STATUSES {
        for row in STATUS_ROWS {
            out.push(measure("S3", &v, x, row, Floor::Graphic.x100()));
        }
    }
    out
}

/// D1 and D3's category test: OKLCH chroma ≥ 0.06 and hue 12°–36°.
#[must_use]
pub fn is_red(rgb: [u8; 3]) -> bool {
    let (c, h) = oklch(linear(rgb));
    c >= DANGER_MIN_CHROMA && (DANGER_HUE.0..=DANGER_HUE.1).contains(&h)
}

/// D1–D3 on the opaque danger and accent fills.
#[must_use]
pub fn danger_failures(danger: [u8; 3], accent: [u8; 3]) -> Vec<SkinRule> {
    let mut out = Vec::new();
    let hue = |rgb| {
        let (c, h) = oklch(linear(rgb));
        (scaled(h, 10.0), scaled(c, 1000.0))
    };
    if !is_red(danger) {
        let (hue_x10, chroma_x1000) = hue(danger);
        out.push(SkinRule::DangerNotRed {
            hue_x10,
            chroma_x1000,
        });
    }
    if is_red(accent) {
        let (hue_x10, chroma_x1000) = hue(accent);
        out.push(SkinRule::AccentIsRed {
            hue_x10,
            chroma_x1000,
        });
    }
    for vision in Vision::ALL {
        let floor = if vision == Vision::Normal {
            MIN_DISTANCE
        } else {
            MIN_CVD_DISTANCE
        };
        let d = distance(danger, accent, vision);
        if d < floor {
            out.push(SkinRule::TooClose {
                rule: "D2".to_owned(),
                a: "danger.fill".to_owned(),
                b: "accent.fill".to_owned(),
                vision: vision.name().to_owned(),
                measured_x10: scaled(d, 10.0),
                required_x10: scaled(floor, 10.0),
            });
        }
    }
    out
}

fn status_failures(base: Base, v: &Variant<'_>) -> Vec<SkinRule> {
    let mut out = Vec::new();
    for (i, a) in STATUSES.iter().enumerate() {
        for b in &STATUSES[i + 1..] {
            let d = distance(v.rgb(a), v.rgb(b), Vision::Normal);
            if d < MIN_DISTANCE {
                out.push(SkinRule::TooClose {
                    rule: "S1".to_owned(),
                    a: (*a).to_owned(),
                    b: (*b).to_owned(),
                    vision: Vision::Normal.name().to_owned(),
                    measured_x10: scaled(d, 10.0),
                    required_x10: scaled(MIN_DISTANCE, 10.0),
                });
            }
        }
    }
    for x in STATUSES.iter().filter(|x| **x != "status.offline") {
        out.extend(cutout_pair(base, v, x).refusal());
    }
    for x in STATUSES {
        for row in STATUS_ROWS {
            out.extend(contrast_rule("S3", v, x, row, Floor::Graphic.x100()));
        }
    }
    out
}

/// Every step-6 failure of a variant on `base`, given its resolved tokens.
#[must_use]
pub fn failures(base: Base, resolved: &[(&TokenEntry, TokenValue)]) -> Vec<SkinRule> {
    let v = Variant::new(resolved);
    let mut out = Vec::new();
    for id in opaque_grounds(resolved) {
        if v.colour(id)[3] != 255 {
            out.push(SkinRule::TranslucentGround {
                token: id.to_owned(),
            });
        }
    }
    let crossover = (1.05f64 * 0.05).sqrt() - 0.05;
    let window = luminance(linear(v.rgb("bg.dark")));
    let dark = matches!(base, Base::Dark | Base::HcDark);
    if (window < crossover) != dark {
        out.push(SkinRule::BaseMismatch {
            declared: base.key().to_owned(),
            token: "bg.dark".to_owned(),
            luminance_x1000: scaled(window, 1000.0),
        });
    }
    for (e, _) in resolved {
        for c in e.constraints {
            let rule = constraint_rule(c.min_ratio_x100);
            for ground in c.on {
                out.extend(contrast_rule(
                    rule,
                    &v,
                    e.id,
                    &Ground::Token(ground),
                    c.min_ratio_x100,
                ));
            }
        }
    }
    for p in PAIRS {
        for g in p.on {
            out.extend(contrast_rule(p.rule, &v, p.fg, g, p.floor.x100()));
        }
    }
    out.extend(skin_model::model_failures(resolved));
    out.extend(danger_failures(v.rgb("danger.fill"), v.rgb("accent.fill")));
    out.extend(status_failures(base, &v));
    out
}

/// Step 6 for the variant whose token file is `entry`: the first failure,
/// as a refusal naming that file.
pub fn validate(
    entry: &str,
    base: Base,
    resolved: &[(&TokenEntry, TokenValue)],
) -> Result<(), SkinError> {
    match failures(base, resolved).into_iter().next() {
        Some(rule) => Err(SkinError::new(entry, rule)),
        None => Ok(()),
    }
}

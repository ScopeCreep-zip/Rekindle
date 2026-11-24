//! A labelled slot's band (plan E7.7.2b, the lead's 2026-11-23 ruling on
//! WCAG 2.2 SC 1.4.3: contrast is measured against what is actually behind
//! the text, so a backing is a remedy only where the art fails): one
//! backing per slot, on its container, never a box per label.
//!
//! SK17 (round 27 screenshots): a header whose art fails its labels is
//! banded across its whole strip, the container's full padding box, so
//! the header reads as one solid bar. A band clipped to the text row in
//! the content box drew a dark box hugging "COMMUNITIES", "CHANNELS" and
//! "# general" inside the lighter header art. The title strip keeps its
//! title row band ([`super::super::title`] measures and bands that row).

use super::super::label_box::{label_box, Line};
use super::super::title::TITLE_REGION;
use super::property;

/// The custom property holding a labelled slot's band colour: set by the
/// skin layer ([`super::labels::label_css`], [`super::title::title_css`])
/// only where the slot's art fails its labels' contrast floor.
#[must_use]
pub fn band_property(slot: &str) -> String {
    format!("{}-band", property(slot, None))
}

/// The `background` of labelled slot `id`'s band: the title strip's
/// title row, every other slot's whole strip.
fn band_background(id: &str) -> String {
    let fill = format!("linear-gradient(var({0}), var({0}))", band_property(id));
    if id != TITLE_REGION {
        return format!("{fill} 0 0 / 100% 100% no-repeat border-box");
    }
    // Quarter px to CSS px.
    let px = |q: u32| f64::from(q) / 4.0;
    let geometry = match label_box(id).line {
        Line::Centred(q) => format!("0 50% / 100% {}px", px(q)),
        Line::Top(q) => format!("0 0 / 100% {}px", px(q)),
        Line::Fill => "0 0 / 100% 100%".to_owned(),
    };
    format!("{fill} {geometry} no-repeat content-box")
}

/// The rules of labelled slot `id`'s `elements` painting its band once,
/// under its labels ([`band_background`]).
pub(super) fn band_rule(id: &str, elements: &[String]) -> String {
    let background = band_background(id);
    let hosts: Vec<String> = elements.iter().map(|e| format!(":where({e})")).collect();
    let before: Vec<String> = elements.iter().map(|e| format!("{e}::before")).collect();
    format!(
        "  {} {{
    position: relative;
    isolation: isolate;
  }}
  {} {{
    content: \"\";
    position: absolute;
    inset: 0;
    z-index: -1;
    padding: inherit;
    border-radius: inherit;
    background: {background};
    pointer-events: none;
  }}
",
        hosts.join(",\n  "),
        before.join(",\n  ")
    )
}

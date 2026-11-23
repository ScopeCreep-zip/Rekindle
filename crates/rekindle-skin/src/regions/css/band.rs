//! A labelled slot's band (plan E7.7.2b, the lead's 2026-11-23 ruling on
//! WCAG 2.2 SC 1.4.3: contrast is measured against what is actually behind
//! the text, so a backing is a remedy only where the art fails): one
//! backing per slot, on its container, behind the text row
//! [`super::super::label_box`] measures, never a box per label.

use super::super::label_box::{label_box, Line};
use super::property;

/// The custom property holding a labelled slot's band colour: set by the
/// skin layer ([`super::labels::label_css`], [`super::title::title_css`])
/// only where the slot's art fails its labels' contrast floor.
#[must_use]
pub fn band_property(slot: &str) -> String {
    format!("{}-band", property(slot, None))
}

/// The rules of labelled slot `id`'s `elements` painting its band once,
/// behind the text row [`label_box`] measures, under its labels.
pub(super) fn band_rule(id: &str, elements: &[String]) -> String {
    let fill = format!("linear-gradient(var({0}), var({0}))", band_property(id));
    // Quarter px to CSS px.
    let px = |q: u32| f64::from(q) / 4.0;
    let geometry = match label_box(id).line {
        Line::Centred(q) => format!("0 50% / 100% {}px", px(q)),
        Line::Top(q) => format!("0 0 / 100% {}px", px(q)),
        Line::Fill => "0 0 / 100% 100%".to_owned(),
    };
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
    background: {fill} {geometry} no-repeat content-box;
    pointer-events: none;
  }}
",
        hosts.join(",\n  "),
        before.join(",\n  ")
    )
}

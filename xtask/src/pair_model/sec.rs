//! The reserved `--sec-*` values of each base (plan E7.7.0b).

use std::collections::BTreeMap;

use crate::css::{self, Block};
use crate::declared_grounds::LIGHT_BASE;

/// The reserved `--sec-*` colours of each base: `:root`, the light base
/// over it, and each with its `prefers-contrast: more` values.
pub(crate) fn sec_bases(global: &str) -> Vec<(&'static str, BTreeMap<String, String>)> {
    let blocks = css::blocks(global);
    let sec: Vec<&Block> = blocks
        .iter()
        .filter(|b| b.is_rule() && b.in_layer("sec"))
        .collect();
    let values = |light: bool, contrast: bool| {
        let applies = |b: &Block| {
            b.selectors()
                .iter()
                .any(|s| s == ":root" || (light && s == LIGHT_BASE))
        };
        let plain = |b: &&&Block| !b.ancestors.iter().any(|a| a.starts_with("@media"));
        let hc = |b: &&&Block| contrast && b.under("prefers-contrast:more");
        let mut out = BTreeMap::new();
        // `:root` first, then the light base over it, then high contrast.
        let layers = [
            sec.iter()
                .filter(|b| plain(b) && b.selectors().iter().any(|s| s == ":root"))
                .collect::<Vec<_>>(),
            sec.iter()
                .filter(|b| plain(b) && light && applies(b))
                .collect(),
            sec.iter().filter(|b| hc(b) && applies(b)).collect(),
        ];
        for block in layers.into_iter().flatten() {
            out.extend(
                block
                    .decls
                    .iter()
                    .filter(|d| d.prop.starts_with("--sec-") && d.value.starts_with('#'))
                    .map(|d| (d.prop.clone(), d.value.clone())),
            );
        }
        out
    };
    vec![
        ("dark", values(false, false)),
        ("light", values(true, false)),
        ("dark+hc", values(false, true)),
        ("light+hc", values(true, true)),
    ]
}

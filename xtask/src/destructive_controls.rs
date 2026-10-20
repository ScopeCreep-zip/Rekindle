//! `check-destructive-controls` (plan E7.7.0, rule D0;
//! `skins/research/d3-distinctness.md` §3.3).
//!
//! WCAG 1.4.1's Understanding note: when content relies on telling a colour
//! apart, "an additional visual indicator will be required regardless of
//! the contrast ratio". "Danger is red" is that case, whatever colours a
//! skin picks, so the cue that a control destroys something cannot be the
//! colour. Over the components that render a Destructive or Protected
//! intent ([`RENDERERS`]; `rekindle-types/src/intent/mod.rs`, `IntentClass`)
//! this gate requires:
//!
//! - destructive styling (any `*-danger` class) only in a static
//!   `class="…"` on an element marked `data-role="destructive"`, and that
//!   mark only on such an element: never a class swapped by an expression;
//! - a non-colour cue on it: a visible verb label (text a sighted user
//!   reads; an `aria-hidden` icon glyph and an `aria-label` do not count).
//!   D0 also allows "the reserved app-owned glyph plus an accessible name"
//!   for an icon-only control; no destructive control is icon-only today,
//!   so no glyph is reserved yet and the label is required. The first
//!   icon-only one reserves its glyph as an `src/icons.ts` id, and this
//!   gate then accepts exactly that glyph with an `aria-label`;
//! - never the default control (Apple HIG: "Don't assign the primary role
//!   to a button that performs a destructive action"): no `type="submit"`,
//!   no `autofocus`;
//! - at most one per view (GNOME HIG: "Each view should only ever include a
//!   single button using either the suggested or destructive styles").
//!
//! A component that commits an intent review (`commitReview(`) and is not
//! in [`RENDERERS`] fails too, so a new intent-composed panel cannot skip
//! the gate.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::jsx::elements;
use crate::walk_source_files;

/// The components that render a Destructive or Protected intent review.
pub(crate) const RENDERERS: &[&str] = &["src/components/common/IntentDialog.tsx"];

/// What marks a component as rendering an intent review.
const REVIEW_COMMIT: &str = "commitReview(";

fn danger_class(value: &str) -> bool {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .any(|w| w.ends_with("-danger"))
}

/// D0 problems in one component.
fn scan(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let mut destructive = 0usize;
    for el in elements(src) {
        let at = format!("{rel}:{}  <{}>", el.line, el.tag);
        let class = el.attrs.get("class").map_or("", String::as_str);
        let marked = el.attrs.get("data-role").map(String::as_str) == Some("\"destructive\"");
        let styled = danger_class(class);
        if styled && class.starts_with('{') {
            hits.push(format!("{at}  destructive styling chosen by an expression; give the destructive control its own element"));
        }
        if styled && !marked {
            hits.push(format!(
                "{at}  destructive styling without `data-role=\"destructive\"`"
            ));
        }
        if !marked {
            continue;
        }
        destructive += 1;
        if !styled {
            hits.push(format!(
                "{at}  `data-role=\"destructive\"` without its destructive class"
            ));
        }
        if el.attrs.get("type").map(String::as_str) == Some("\"submit\"") {
            hits.push(format!(
                "{at}  a destructive control is the form's default (`type=\"submit\"`)"
            ));
        }
        if el.attrs.contains_key("autofocus") || el.attrs.contains_key("autoFocus") {
            hits.push(format!(
                "{at}  a destructive control takes focus by default"
            ));
        }
        if el.visible_text().is_empty() {
            hits.push(format!(
                "{at}  no non-colour cue: give it a visible verb label (no glyph is reserved yet)"
            ));
        }
    }
    if destructive > 1 {
        hits.push(format!(
            "{rel}  {destructive} destructive controls in one view (at most one)"
        ));
    }
    hits
}

pub(crate) fn check_destructive_controls(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    for rel in RENDERERS {
        let path = root.join(rel);
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(scan(rel, &src));
    }
    for path in walk_source_files(&root.join("src"), &["tsx"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        if src.contains(REVIEW_COMMIT) && !RENDERERS.contains(&rel.as_str()) {
            hits.push(format!(
                "{rel}  commits an intent review but is not in RENDERERS \
                 (xtask/src/destructive_controls.rs)"
            ));
        }
    }
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} destructive-control violation(s) (rule D0: a visible verb \
         label, never the default, at most one per view).",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"const D = () => {
  const [s] = createSignal<string>("");
  return (
    <form onSubmit={submit}>
      <button type="button" class="sec-btn-secondary" onClick={() => void cancel()}>Cancel</button>
      <Show when={warning()} fallback={<button type="submit" class="sec-btn-commit">{label()}</button>}>
        <button type="button" class="sec-btn-commit-danger" data-role="destructive" onClick={commit}>
          {label()}
        </button>
      </Show>
    </form>
  );
};"#;

    #[test]
    fn the_reviewed_shape_passes() {
        let hits = scan("D.tsx", GOOD);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn a_swapped_class_fails() {
        let swap = r#"<button type="submit" class={warning() ? "form-btn-danger" : "form-btn-primary"}>{label()}</button>"#;
        let hits = scan("D.tsx", swap);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits[0].contains("chosen by an expression"));
    }

    #[test]
    fn a_default_unlabelled_or_repeated_destructive_control_fails() {
        let default = r#"<button type="submit" class="sec-btn-commit-danger" data-role="destructive">Delete</button>"#;
        assert!(scan("D.tsx", default)[0].contains("default"));
        let focused = r#"<button autofocus class="sec-btn-commit-danger" data-role="destructive">Delete</button>"#;
        assert!(scan("D.tsx", focused)[0].contains("focus"));
        let bare = r#"<button class="sec-btn-commit-danger" data-role="destructive" />"#;
        assert!(scan("D.tsx", bare)[0].contains("non-colour cue"));
        // An accessible name alone, or an icon a sighted user cannot read
        // as a verb, is no cue while no glyph is reserved.
        let named = r#"<button class="sec-btn-commit-danger" data-role="destructive" aria-label="Delete channel" />"#;
        assert!(scan("D.tsx", named)[0].contains("visible verb label"));
        let icon = r#"<button class="sec-btn-commit-danger" data-role="destructive" aria-label="Delete"><span class="nf-icon" aria-hidden="true">{ICON_TRASH}</span></button>"#;
        assert!(scan("D.tsx", icon)[0].contains("visible verb label"));
        let two = format!("{default}{default}").replace("type=\"submit\" ", "");
        assert!(scan("D.tsx", &two)
            .iter()
            .any(|h| h.contains("2 destructive controls")));
    }

    #[test]
    fn a_mark_without_styling_or_styling_without_mark_fails() {
        let unmarked = r#"<button class="context-menu-item-danger">Remove</button>"#;
        assert!(scan("D.tsx", unmarked)[0].contains("without `data-role"));
        let unstyled = r#"<button class="sec-btn-commit" data-role="destructive">Remove</button>"#;
        assert!(scan("D.tsx", unstyled)[0].contains("without its destructive class"));
    }
}

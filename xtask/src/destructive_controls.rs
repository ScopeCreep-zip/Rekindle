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
//! - a non-colour cue on it: visible content (the verb label) or an
//!   `aria-label` (a reserved glyph's accessible name);
//! - never the default control (Apple HIG: "Don't assign the primary role
//!   to a button that performs a destructive action"): no `type="submit"`,
//!   no `autofocus`;
//! - at most one per view (GNOME HIG: "Each view should only ever include a
//!   single button using either the suggested or destructive styles").
//!
//! A component that commits an intent review (`commitReview(`) and is not
//! in [`RENDERERS`] fails too, so a new intent-composed panel cannot skip
//! the gate.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::walk_source_files;

/// The components that render a Destructive or Protected intent review.
pub(crate) const RENDERERS: &[&str] = &["src/components/common/IntentDialog.tsx"];

/// What marks a component as rendering an intent review.
const REVIEW_COMMIT: &str = "commitReview(";

/// One JSX opening tag.
#[derive(Debug)]
struct Element {
    tag: String,
    line: usize,
    /// Attribute name → raw value (`"…"` or `{…}`, or empty when bare).
    attrs: BTreeMap<String, String>,
    /// The text between the tag and its closing tag, `None` if self-closing.
    children: Option<String>,
}

/// Whether a `<` at `at` opens a JSX tag rather than a comparison or a
/// generic (`createSignal<Stage>(`): the previous non-blank character is
/// not part of an expression operand.
fn opens_tag(src: &str, at: usize) -> bool {
    let next = src[at + 1..].chars().next();
    if !next.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let prev = src[..at].trim_end().chars().next_back();
    !prev.is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ')' | ']' | '.'))
}

/// The end of an opening tag starting at `at` (index of its `>`).
fn tag_end(src: &str, at: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for (i, c) in src[at..].char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' | '`' => quote = Some(c),
                '{' => depth += 1,
                '}' => depth -= 1,
                '>' if depth == 0 => return Some(at + i),
                _ => {}
            },
        }
    }
    None
}

fn parse_attrs(inner: &str) -> BTreeMap<String, String> {
    let mut attrs = BTreeMap::new();
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !(chars[i].is_ascii_alphabetic() || chars[i] == '-') {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '-' | ':' | '_'))
        {
            i += 1;
        }
        let name: String = chars[start..i].iter().collect();
        let mut value = String::new();
        if chars.get(i) == Some(&'=') {
            i += 1;
            let open = chars.get(i).copied();
            let (close, nests) = match open {
                Some('"') => ('"', false),
                Some('\'') => ('\'', false),
                Some('{') => ('}', true),
                _ => (' ', false),
            };
            let vstart = i;
            let mut depth = 0i32;
            while i < chars.len() {
                let c = chars[i];
                if nests && c == '{' {
                    depth += 1;
                } else if c == close && (!nests || depth == 1) && i > vstart {
                    i += 1;
                    break;
                } else if nests && c == '}' {
                    depth -= 1;
                }
                i += 1;
            }
            value = chars[vstart..i.min(chars.len())].iter().collect();
        }
        attrs.insert(name, value);
    }
    attrs
}

/// Every JSX element of a TSX source.
fn elements(src: &str) -> Vec<Element> {
    let mut out = Vec::new();
    for (at, _) in src.match_indices('<') {
        if !opens_tag(src, at) {
            continue;
        }
        let Some(end) = tag_end(src, at) else {
            continue;
        };
        let inner = &src[at + 1..end];
        let tag: String = inner
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
            .collect();
        let self_closing = inner.trim_end().ends_with('/');
        let children = (!self_closing).then(|| closing_content(src, end + 1, &tag));
        out.push(Element {
            line: src[..at].matches('\n').count() + 1,
            attrs: parse_attrs(&inner[tag.len()..]),
            tag,
            children,
        });
    }
    out
}

/// The text from `from` to the matching `</tag>`.
fn closing_content(src: &str, from: usize, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1;
    let mut i = from;
    while let Some(next_close) = src[i..].find(&close) {
        let opens = src[i..i + next_close]
            .match_indices(&open)
            .filter(|(o, _)| {
                src[i + o + open.len()..].starts_with(|c: char| c.is_whitespace() || c == '>')
            })
            .count();
        depth += opens;
        depth -= 1;
        if depth == 0 {
            return src[from..i + next_close].to_string();
        }
        i += next_close + close.len();
    }
    src[from..].to_string()
}

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
        let labelled = el.attrs.contains_key("aria-label")
            || el.children.as_deref().is_some_and(|c| !c.trim().is_empty());
        if !labelled {
            hits.push(format!(
                "{at}  no non-colour cue: give it a verb label or an `aria-label`"
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
        "{} destructive-control violation(s) (rule D0: a verb label or \
         reserved glyph, never the default, at most one per view).",
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
        let glyph = r#"<button class="sec-btn-commit-danger" data-role="destructive" aria-label="Delete channel" />"#;
        assert!(scan("D.tsx", glyph).is_empty());
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

    #[test]
    fn generics_and_comparisons_are_not_tags() {
        let src = "const a = b < c; const [x] = createSignal<Stage>(\"intro\"); type T = Record<string, number>;";
        assert!(elements(src).is_empty());
        let nested = "<div class=\"a\"><div>inner</div>tail</div>";
        let els = elements(nested);
        assert_eq!(els[0].children.as_deref(), Some("<div>inner</div>tail"));
    }
}

//! `check-sec-surfaces` (plan E7.7.0; closed against the bypasses the
//! 2026-10-20 audit found, O-3): security surfaces are unskinnable.
//!
//! A skin is colour and bitmaps (E7.7.1). If a component that asks for a
//! deliberate decision drew its buttons with the themeable `.form-btn-*`
//! classes, a colour-only skin could make a destructive commit look like a
//! safe one (`IntentDialog` used to swap `.form-btn-danger` and
//! `.form-btn-primary` on one element); if the text the decision rests on
//! read a themeable token, a skin could paint it in its own ground's colour
//! and hide it. So each surface's **security-critical** elements (the
//! commit controls, and the text a user reads or compares before deciding:
//! the review's subject, facts, escalations and consequences, a key
//! fingerprint, a safety number, a pairing code and its QR) carry only
//! reserved `.sec-*` classes, drawn in `@layer sec` on a fixed ground. The
//! dialog chrome around them (title, frame, overlay, form fields, error
//! line) stays themeable. The gate, over the components (here) and the
//! stylesheets ([`crate::sec_styles`]):
//!
//! - every component in [`SURFACES`] exists, uses every security class its
//!   row names, uses no `form-btn-*` class and reads no custom property
//!   but a reserved `--sec-*` one;
//! - an element there with a `sec-*` class (in `class`, `className` or as
//!   a `classList` key) carries no other class, and its `class` is a static
//!   string: no expression, no template literal, no `classList`,
//!   `className`, `style`, `hidden` or prop spread, each of which can add a
//!   themeable class or hide the element;
//! - every element inside it obeys the same, and the only components it
//!   renders are Solid's control flow and the other listed surfaces;
//! - a `sec-*` name is written in a surface only in a static `class`
//!   attribute (never in a string a helper hands to one), and in no other
//!   `.ts` or `.tsx` file under `src/` (comments aside);
//! - the stylesheet rules of [`crate::sec_styles`].

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::css;
use crate::jsx::{elements, strip_comments, Element};
use crate::sec_styles;
use crate::walk_source_files;

/// The security-surface components and the security-critical classes each
/// must use. The incoming session reset is a Host review rendered by
/// `IntentDialog` (plan E2.1); the safety number and the key-trust badge
/// are what two people compare (plan E2.3); E7.7.2's skin-error banner and
/// its keep-or-revert control join this list in their step.
pub(crate) const SURFACES: &[(&str, &[&str])] = &[
    (
        "src/components/common/IntentDialog.tsx",
        &[
            "sec-review",
            "sec-review-subject",
            "sec-review-facts",
            "sec-review-escalation",
            "sec-review-meta",
            "sec-review-ack",
            "sec-btn-commit",
            "sec-btn-commit-danger",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/buddy-list/DeepLinkConsentDialog.tsx",
        &[
            "sec-review",
            "sec-review-fact",
            "sec-fingerprint",
            "sec-fingerprint-value",
            "sec-btn-commit",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/settings/AddDeviceModal.tsx",
        &[
            "sec-pairing-qr",
            "sec-pairing-code",
            "sec-btn-commit",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/profile/SafetyNumberPanel.tsx",
        &[
            "sec-safety-number",
            "sec-safety-number-digits",
            "sec-safety-number-qr",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/profile/VerifiedBadge.tsx",
        &[
            "sec-verified-badge",
            "sec-key-changed-badge",
            "sec-badge-icon",
        ],
    ),
];

/// Solid's components that only choose what renders and draw nothing.
const CONTROL_FLOW: &[&str] = &["Show", "For", "Index", "Switch", "Match"];

/// Attributes that can add a class to an element or hide it.
const RESTYLING_ATTRS: &[&str] = &["classList", "className", "style", "hidden"];

pub(crate) fn is_sec_class(class: &str) -> bool {
    class.starts_with("sec-")
}

/// Every `sec-…` name in `text` with its line: a `sec-` that no name
/// character precedes (`spec-x`, `--sec-focus` are not one), and the name
/// characters after it.
pub(crate) fn sec_names(text: &str) -> Vec<(usize, String)> {
    text.match_indices("sec-")
        .filter(|(at, _)| {
            !text[..*at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
        .map(|(at, _)| {
            let name = text[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            (text[..at].matches('\n').count() + 1, name)
        })
        .collect()
}

fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The components a surface imports from another listed surface (`import
/// VerifiedBadge from "./VerifiedBadge"`): they may render inside a
/// security-critical element, since the gate checks them too.
fn surface_imports(rel: &str, src: &str) -> BTreeSet<String> {
    let dir = Path::new(rel).parent().unwrap_or_else(|| Path::new(""));
    src.lines()
        .filter_map(|line| {
            let (name, from) = line.trim().strip_prefix("import ")?.split_once(" from ")?;
            let name = name.trim();
            if !name.starts_with(|c: char| c.is_ascii_uppercase())
                || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return None;
            }
            let spec = from
                .trim()
                .trim_end_matches(';')
                .trim_matches(|c| c == '"' || c == '\'');
            if !spec.starts_with('.') {
                return None;
            }
            let target = normalise(&dir.join(spec)).with_extension("tsx");
            let target = target.to_string_lossy().replace('\\', "/");
            SURFACES
                .iter()
                .any(|(s, _)| *s == target)
                .then(|| name.to_string())
        })
        .collect()
}

/// Why `el` (a security-critical element or one inside it) could carry a
/// themeable class or be hidden.
fn restyling(el: &Element) -> Vec<String> {
    let mut why = Vec::new();
    if el.class_has_template() {
        why.push("builds its class with a template literal".to_string());
    } else if !el.class_is_static() {
        why.push("chooses its class with an expression".to_string());
    }
    for attr in RESTYLING_ATTRS {
        if el.attrs.contains_key(*attr) {
            why.push(format!("carries `{attr}`, which can restyle or hide it"));
        }
    }
    if el.spread {
        why.push("spreads props, which can add a class".to_string());
    }
    for class in el.classes() {
        if !is_sec_class(&class) {
            why.push(format!("also carries the themeable class `{class}`"));
        }
    }
    why
}

fn push_unique(hits: &mut Vec<String>, hit: String) {
    if !hits.contains(&hit) {
        hits.push(hit);
    }
}

/// `src` with the value of every static `class="…"` blanked, so what is
/// left names a security class only where the gate cannot see it.
fn without_static_classes(src: &str, els: &[Element]) -> String {
    let mut out = src.to_string();
    for el in els.iter().filter(|e| e.class_is_static()) {
        let tag = &src[el.open.clone()];
        let Some(at) = tag
            .match_indices("class=")
            .map(|(i, _)| i)
            .find(|i| tag[..*i].ends_with(char::is_whitespace))
        else {
            continue;
        };
        let value = at + "class=".len();
        let Some(quote) = tag[value..].chars().next() else {
            continue;
        };
        let Some(len) = tag[value + 1..].find(quote) else {
            continue;
        };
        let range = el.open.start + value..el.open.start + value + len + 2;
        out.replace_range(range.clone(), &" ".repeat(range.len()));
    }
    out
}

/// What is wrong with the elements inside the security-critical `el`.
fn inner_problems(
    rel: &str,
    el: &Element,
    els: &[Element],
    allowed: &BTreeSet<String>,
) -> Vec<String> {
    let mut hits = Vec::new();
    for inner in els
        .iter()
        .filter(|d| d.span.start >= el.open.end && d.span.end <= el.span.end)
    {
        if inner.classes().iter().any(|c| is_sec_class(c)) {
            continue; // checked as a security-critical element itself
        }
        let mut why = restyling(inner);
        if inner.tag.starts_with(|c: char| c.is_ascii_uppercase())
            && !CONTROL_FLOW.contains(&inner.tag.as_str())
            && !allowed.contains(&inner.tag)
        {
            why.push(
                "is a component that is not a listed surface, so it can draw any class".into(),
            );
        }
        for w in why {
            hits.push(format!(
                "{rel}:{}  <{}> sits in the security-critical <{}> (line {}) but {w}",
                inner.line, inner.tag, el.tag, el.line
            ));
        }
    }
    hits
}

/// The form-button and token problems of a surface, line by line.
fn line_problems(rel: &str, src: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for (i, line) in src.lines().enumerate() {
        if line.contains("form-btn-") {
            hits.push(format!(
                "{rel}:{}  uses a themeable `.form-btn-*` class; use `.sec-btn-*`",
                i + 1
            ));
        }
        for name in css::var_refs(line) {
            if !name.starts_with("--sec-") {
                hits.push(format!(
                    "{rel}:{}  reads the themeable token `{name}`",
                    i + 1
                ));
            }
        }
    }
    hits
}

/// Problems in one security-surface component; adds the security classes
/// it uses to `used`.
fn scan_component(
    rel: &str,
    src: &str,
    required: &[&str],
    used: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut hits = line_problems(rel, src);
    let els = elements(src);
    let allowed = surface_imports(rel, src);
    let mut here = BTreeSet::new();
    for el in &els {
        let classes = el.classes();
        if !classes.iter().any(|c| is_sec_class(c)) {
            continue;
        }
        here.extend(classes.into_iter().filter(|c| is_sec_class(c)));
        for why in restyling(el) {
            push_unique(
                &mut hits,
                format!(
                    "{rel}:{}  <{}> is security-critical but {why}",
                    el.line, el.tag
                ),
            );
        }
        for hit in inner_problems(rel, el, &els, &allowed) {
            push_unique(&mut hits, hit);
        }
    }
    let rest = strip_comments(&without_static_classes(src, &els));
    for (line, name) in sec_names(&rest) {
        push_unique(
            &mut hits,
            format!(
                "{rel}:{line}  names the security class `{name}` outside a static `class` \
                 attribute; write it only on its element"
            ),
        );
    }
    for class in required {
        if !here.contains(*class) {
            hits.push(format!(
                "{rel}  does not use its security class `{class}` (xtask/src/sec_surfaces.rs)"
            ));
        }
    }
    used.extend(here);
    hits
}

/// A file outside [`SURFACES`] that names a security class anywhere but in
/// a comment (a class attribute, a `classList` key, a string handed to
/// `classList.add`).
fn stray_sec_names(rel: &str, src: &str) -> Vec<String> {
    sec_names(&strip_comments(src))
        .into_iter()
        .map(|(line, name)| {
            format!(
                "{rel}:{line}  names the security class `{name}` but is not in SURFACES \
                 (xtask/src/sec_surfaces.rs)"
            )
        })
        .collect()
}

pub(crate) fn check_sec_surfaces(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    let mut used = BTreeSet::new();
    for (rel, required) in SURFACES {
        let path = root.join(rel);
        if !path.exists() {
            hits.push(format!(
                "{rel}  is listed as a security surface but does not exist"
            ));
            continue;
        }
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(scan_component(rel, &src, required, &mut used));
    }
    for path in walk_source_files(&root.join("src"), &["ts", "tsx"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string()
            .replace('\\', "/");
        if SURFACES.iter().any(|(s, _)| *s == rel) {
            continue;
        }
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(stray_sec_names(&rel, &src));
    }
    for (rel, src) in css::stylesheets(root)? {
        hits.extend(sec_styles::scan(&rel, &src));
    }
    if hits.is_empty() {
        println!(
            "  {} security classes on {} surfaces",
            used.len(),
            SURFACES.len()
        );
        return Ok(());
    }
    for h in &hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!(
        "{} security-surface violation(s). Security-critical elements carry \
         only the `.sec-*` classes of `@layer sec` \
         (src/styles/theme/16-security-surfaces.css), which read only the \
         reserved `--sec-*` tokens.",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str, required: &[&str]) -> Vec<String> {
        scan_component(
            "src/components/profile/D.tsx",
            src,
            required,
            &mut BTreeSet::new(),
        )
    }

    fn has(hits: &[String], text: &str) -> bool {
        hits.iter().any(|h| h.contains(text))
    }

    #[test]
    fn a_surface_with_a_form_button_or_a_theme_token_fails() {
        let ok = r#"<button class="sec-btn-commit-danger" data-role="destructive">Delete</button>
            <rect class="status-dot-cutout" />"#;
        assert!(scan(ok, &["sec-btn-commit-danger"]).is_empty());
        let swap =
            r#"<button class={warning() ? "form-btn-danger" : "form-btn-primary"}>Go</button>"#;
        assert_eq!(scan(swap, &[]).len(), 1);
        let token = r#"<div data-c="var(--color-xfire-text)" />"#;
        let hits = scan(token, &[]);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("--color-xfire-text"), "{hits:?}");
        assert!(scan(r#"<i data-x="fill: var(--sec-focus)" />"#, &[]).is_empty());
    }

    #[test]
    fn security_text_carries_only_its_security_classes() {
        let mixed = r#"<p class="sec-fingerprint consent-dialog-key">Key <code class="sec-fingerprint-value">ab</code></p>"#;
        let hits = scan(mixed, &["sec-fingerprint", "sec-fingerprint-value"]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].contains("themeable class `consent-dialog-key`"));
        let gone = r#"<p class="consent-dialog-key">Key <code>ab</code></p>"#;
        let hits = scan(gone, &["sec-fingerprint"]);
        assert!(hits[0].contains("does not use its security class `sec-fingerprint`"));
    }

    #[test]
    fn every_element_inside_a_security_element_is_checked() {
        let icon = r#"<span class="sec-verified-badge"><span class="nf-icon" aria-hidden="true">x</span> Verified</span>"#;
        let hits = scan(icon, &[]);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            has(&hits, "sits in the security-critical <span>"),
            "{hits:?}"
        );
        assert!(has(&hits, "themeable class `nf-icon`"), "{hits:?}");
        let deep = r#"<div class="sec-review"><Show when={x()}><ul><li class={dim()}>a</li></ul></Show></div>"#;
        assert!(has(
            &scan(deep, &[]),
            "chooses its class with an expression"
        ));
        let hidden = r#"<div class="sec-review"><p style={{ opacity: 0 }}>fact</p></div>"#;
        assert!(has(&scan(hidden, &[]), "carries `style`"));
        let component = r#"<div class="sec-review"><Tooltip text="x" /></div>"#;
        assert!(has(&scan(component, &[]), "<Tooltip> sits in"));
        let listed = "import VerifiedBadge from \"./VerifiedBadge\";\n<div class=\"sec-safety-number\"><VerifiedBadge view={v()} /><span aria-hidden=\"true\">1</span></div>";
        let hits = scan(listed, &[]);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn class_lists_templates_spreads_and_hiding_attributes_fail() {
        let list = r#"<div class="sec-review" classList={{ dim: off() }} />"#;
        assert!(has(&scan(list, &[]), "carries `classList`"));
        let keyed = r#"<div classList={{ "sec-review": on() }} />"#;
        assert!(has(&scan(keyed, &[]), "carries `classList`"));
        let template = r"<div class={`sec-review ${extra()}`} />";
        assert!(has(&scan(template, &[]), "template literal"));
        let spread = r#"<button class="sec-btn-commit" {...rest}>Go</button>"#;
        assert!(has(&scan(spread, &[]), "spreads props"));
        let hidden = r#"<p class="sec-review-fact" hidden={!x()}>a</p>"#;
        assert!(has(&scan(hidden, &[]), "carries `hidden`"));
    }

    #[test]
    fn a_security_name_outside_a_static_class_attribute_fails() {
        let helper = "const cls = () => \"sec-review\";\n// sec-review in a comment is fine\n<div class={cls()} />";
        let hits = scan(helper, &[]);
        assert!(
            has(
                &hits,
                "D.tsx:1  names the security class `sec-review` outside"
            ),
            "{hits:?}"
        );
        assert!(!has(&hits, "D.tsx:2 "), "{hits:?}");
        let ok = "/** draws `.sec-review` */\n<div class=\"sec-review\">a</div>";
        assert!(scan(ok, &["sec-review"]).is_empty());
    }

    #[test]
    fn any_other_file_naming_a_security_class_fails() {
        assert_eq!(
            stray_sec_names("X.tsx", r#"<b class="sec-review" />"#).len(),
            1
        );
        let ts = "el.classList.add(\"sec-btn-commit\");\n// sec-x\nconst v = \"--sec-focus\";";
        let hits = stray_sec_names("x.ts", ts);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(hits[0].starts_with("x.ts:1 "), "{hits:?}");
        assert!(stray_sec_names("X.tsx", r#"<b class="review spec-x" />"#).is_empty());
    }

    #[test]
    fn imports_resolve_to_listed_surfaces_only() {
        let src = "import VerifiedBadge from \"./VerifiedBadge\";\nimport Modal from \"../common/Modal\";";
        let names = surface_imports("src/components/profile/SafetyNumberPanel.tsx", src);
        assert_eq!(names.into_iter().collect::<Vec<_>>(), vec!["VerifiedBadge"]);
    }
}

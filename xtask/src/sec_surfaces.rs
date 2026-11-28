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
        "src/components/buddy-list/PairingLinkNotice.tsx",
        &["sec-review", "sec-review-fact", "sec-btn-secondary"],
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
        "src/components/buddy-list/FriendCodeCard.tsx",
        &[
            "sec-friend-code",
            "sec-friend-code-url",
            "sec-friend-code-qr",
            "sec-btn-secondary",
        ],
    ),
    (
        "src/components/settings/SkinTrialPrompt.tsx",
        &[
            "sec-review",
            "sec-review-subject",
            "sec-review-fact",
            "sec-btn-commit",
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
/// `ErrorBoundary` renders its children or its `fallback`, whose elements
/// sit inside the security element and are checked like any other.
const CONTROL_FLOW: &[&str] = &["Show", "For", "Index", "Switch", "Match", "ErrorBoundary"];

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
#[path = "sec_surfaces_tests.rs"]
mod tests;

//! Plan E7.7: a pack's art paints only the windows the pack skins
//! (`regions::css::scope`). Every built-in's served sheet is walked rule by
//! rule: a rule that draws pack art (a `skin://` image, or a region or
//! icon property) must be scoped to a skinned window's root, so none can
//! match Settings (`.app-frame[data-window="settings"]`), whose skin list
//! once showed Combat's camo frame and Symbiosis's buttons.

use rekindle_skin::regions::css::scope::{skinned_windows, PACK_WINDOWS, UNSKINNED_WINDOWS};
use rekindle_skin::{builtin, serve, validate};

/// The innermost rules of `css` as (selector, body), at-rule blocks walked.
fn leaf_rules(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<(String, usize, bool)> = Vec::new();
    let mut seg = 0;
    for (i, c) in css.char_indices() {
        match c {
            '{' => {
                if let Some(top) = stack.last_mut() {
                    top.2 = true;
                }
                stack.push((css[seg..i].trim().to_owned(), i + 1, false));
                seg = i + 1;
            }
            '}' => {
                let (sel, start, nested) = stack.pop().expect("balanced braces");
                if !nested {
                    out.push((sel, css[start..i].to_owned()));
                }
                seg = i + 1;
            }
            ';' => seg = i + 1,
            _ => {}
        }
    }
    out
}

/// Whether a rule body draws pack art.
fn is_art(body: &str) -> bool {
    body.contains("skin://")
        || body.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with("--skin-region") || l.starts_with("--skin-icon")
        })
}

/// The selector list's top-level parts (commas inside `:is(…)` kept).
fn parts(selector: &str) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0usize, 0, Vec::new());
    for (i, c) in selector.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(selector[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(selector[start..].trim());
    out
}

/// The `data-window` kinds a selector part names.
fn kinds(part: &str) -> Vec<&str> {
    part.split("[data-window=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect()
}

/// Whether a selector part can only match inside a skinned window: it
/// names a skinned kind and no other, or a pack-laid window or popup
/// (`data-skin-window` / `data-skin-popup`, which only skinned windows
/// carry).
fn scoped(part: &str) -> bool {
    if part.contains("[data-skin-window") || part.contains("[data-skin-popup") {
        return true;
    }
    let ks = kinds(part);
    !ks.is_empty() && ks.iter().all(|k| skinned_windows().any(|s| s == *k))
}

#[test]
fn the_kind_table_leaves_settings_unskinned() {
    for k in UNSKINNED_WINDOWS {
        assert!(skinned_windows().all(|s| s != k), "{k} is skinned");
    }
    let main = PACK_WINDOWS.iter().find(|(c, _)| *c == "MainWindow");
    assert_eq!(main.map(|(_, k)| *k), Some(&["buddy-list"][..]));
}

#[test]
fn no_builtin_art_rule_can_match_settings() {
    for pack in &builtin::PACKS {
        let c = validate::run(pack.bytes).unwrap_or_else(|e| panic!("{}: {e:?}", pack.id));
        let sheet = serve::stylesheet(&c.bytes, &c.address()).expect("a stylesheet");
        let art: Vec<_> = leaf_rules(&sheet.css)
            .into_iter()
            .filter(|(_, body)| is_art(body))
            .collect();
        assert!(!art.is_empty(), "{}: no art", pack.id);
        for (sel, _) in &art {
            for part in parts(sel) {
                assert!(scoped(part), "{}: unscoped art rule `{part}`", pack.id);
                assert!(!kinds(part).contains(&"settings"), "{}: `{part}`", pack.id);
            }
        }
        for kind in ["buddy-list", "chat", "group-chat", "community", "popup"] {
            let found = art
                .iter()
                .any(|(sel, _)| parts(sel).iter().any(|p| kinds(p).contains(&kind)));
            assert!(found, "{}: no art reaches {kind}", pack.id);
        }
    }
}

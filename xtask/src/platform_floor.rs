//! `check-platform-floor` and `check-built-css` (plan E7.10's Verify,
//! automated by the 2026-10-20 audit, O-1 and O-5).
//!
//! The UI's CSS needs Tailwind 4's engines: Safari 16.4, Chrome 111 and
//! Firefox 128 (`tailwindcss.com/docs/compatibility`), and it depends on
//! `@layer` (Safari 15.4, WebKitGTK 2.36, Chromium 99), `@property`
//! (Safari 16.4, WebKitGTK 2.40, Chromium 85) and `color-mix()` (Safari
//! 16.2, Chromium 111), none of which esbuild or Lightning CSS lowers. The
//! floor is declared in four places that must agree, and
//! `check-platform-floor` reads each:
//!
//! - `src-tauri/tauri.conf.json`: `bundle.macOS.minimumSystemVersion`
//!   ([`MACOS_FLOOR`], the oldest macOS Apple still patches), the `.deb`
//!   and `.rpm` WebKitGTK depends ([`DEB_WEBKIT`], [`RPM_WEBKIT`]);
//! - `vite.config.ts`: the build targets, exactly [`VITE_TARGETS`];
//! - `docs/contributor/release-process.md` ("Platform floor") names every
//!   value above.
//!
//! `check-built-css <css-or-dir>…` reads the CSS a `vite build` wrote
//! (a file, or every `.css` in a directory such as `dist/assets`). It runs
//! only when given a built path, so `cargo xtask check` stays build-free:
//! every layer `global.css` declares is present, first opened in the
//! declared order (Lightning CSS drops the leading `@layer` statement and
//! keeps the order by first appearance), and every rule outside a layer
//! sets custom properties only (an unlayered declaration would beat
//! `@layer sec`).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::css;

pub(crate) const MACOS_FLOOR: &str = "14.0";
pub(crate) const DEB_WEBKIT: &str = "libwebkit2gtk-4.1-0 (>= 2.40)";
pub(crate) const RPM_WEBKIT: &str = "webkit2gtk4.1 >= 2.40";
pub(crate) const VITE_TARGETS: &[&str] = &["chrome111", "safari16.4"];

const CONF: &str = "src-tauri/tauri.conf.json";
const VITE: &str = "vite.config.ts";
const RELEASE_DOC: &str = "docs/contributor/release-process.md";
const GLOBAL_CSS: &str = "src/styles/global.css";

fn depends(conf: &serde_json::Value, format: &str) -> Vec<String> {
    conf.pointer(&format!("/bundle/linux/{format}/depends"))
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The floor problems of the bundle config.
fn conf_problems(conf: &serde_json::Value) -> Vec<String> {
    let mut hits = Vec::new();
    let macos = conf
        .pointer("/bundle/macOS/minimumSystemVersion")
        .and_then(serde_json::Value::as_str);
    if macos != Some(MACOS_FLOOR) {
        hits.push(format!(
            "{CONF}  bundle.macOS.minimumSystemVersion is {macos:?}, the floor is \"{MACOS_FLOOR}\""
        ));
    }
    for (format, want) in [("deb", DEB_WEBKIT), ("rpm", RPM_WEBKIT)] {
        if !depends(conf, format).iter().any(|d| d == want) {
            hits.push(format!(
                "{CONF}  bundle.linux.{format}.depends lacks the WebKitGTK floor `{want}`"
            ));
        }
    }
    hits
}

/// The engine targets a Vite config names (`"chrome111"`, `"safari16.4"`).
fn vite_targets(src: &str) -> Vec<String> {
    let mut out: Vec<String> = src
        .split('"')
        .skip(1)
        .step_by(2)
        .filter(|s| {
            ["chrome", "safari", "firefox", "edge", "es20"]
                .iter()
                .any(|p| {
                    s.strip_prefix(p).is_some_and(|v| {
                        !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.')
                    })
                })
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn vite_problems(src: &str) -> Vec<String> {
    let got = vite_targets(src);
    if got == VITE_TARGETS {
        return Vec::new();
    }
    vec![format!(
        "{VITE}  build.target names {got:?}; the floor is {VITE_TARGETS:?} (Tailwind 4 and \
         `color-mix()` need Chrome 111 and Safari 16.4)"
    )]
}

fn doc_problems(doc: &str) -> Vec<String> {
    [MACOS_FLOOR, DEB_WEBKIT, RPM_WEBKIT]
        .iter()
        .chain(VITE_TARGETS)
        .filter(|v| !doc.contains(**v))
        .map(|v| format!("{RELEASE_DOC}  \"Platform floor\" does not name `{v}`"))
        .collect()
}

fn report(hits: &[String], what: &str) -> Result<()> {
    if hits.is_empty() {
        return Ok(());
    }
    for h in hits {
        println!("  ✗ {h}");
    }
    Err(anyhow!("{} {what} violation(s)", hits.len()))
}

fn read(root: &Path, rel: &str) -> Result<String> {
    std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
}

pub(crate) fn check_platform_floor(root: &Path) -> Result<()> {
    let conf: serde_json::Value =
        serde_json::from_str(&read(root, CONF)?).with_context(|| format!("parsing {CONF}"))?;
    let mut hits = conf_problems(&conf);
    hits.extend(vite_problems(&read(root, VITE)?));
    hits.extend(doc_problems(&read(root, RELEASE_DOC)?));
    report(&hits, "platform-floor")
}

/// The layer names of the first `@layer a, b, …;` statement of a sheet.
fn declared_layers(src: &str) -> Vec<String> {
    let text = css::strip_comments(src);
    text.split(';')
        .map(str::trim)
        .find_map(|stmt| stmt.strip_prefix("@layer "))
        .map(|names| names.split(',').map(|n| n.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Every layer name in the order a stylesheet first opens it.
fn layer_order(built: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    for (at, _) in built.match_indices("@layer") {
        let rest = &built[at + "@layer".len()..];
        let end = rest.find(['{', ';']).unwrap_or(rest.len());
        for name in rest[..end]
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            if !order.iter().any(|o| o == name) {
                order.push(name.to_string());
            }
        }
    }
    order
}

/// The problems of one built stylesheet against the declared layers.
fn built_problems(rel: &str, built: &str, declared: &[String]) -> Vec<String> {
    let mut hits = Vec::new();
    let order: Vec<String> = layer_order(built)
        .into_iter()
        .filter(|l| declared.contains(l))
        .collect();
    if order != declared {
        hits.push(format!(
            "{rel}  opens the layers as {order:?}; global.css declares {declared:?}"
        ));
    }
    for block in css::blocks(built) {
        let layered = block.ancestors.iter().any(|p| p.starts_with("@layer"));
        let inert = block
            .ancestors
            .iter()
            .any(|p| p.starts_with("@keyframes") || p.starts_with("@font-face"));
        if !block.is_rule() || layered || inert {
            continue;
        }
        if let Some(d) = block.decls.iter().find(|d| !d.prop.starts_with("--")) {
            hits.push(format!(
                "{rel}  `{}` sets `{}` outside every layer, so it beats `@layer sec`",
                block.prelude, d.prop
            ));
        }
    }
    hits
}

/// The stylesheets a built path names: the file, or every `.css` in a directory.
fn built_sheets(path: &Path) -> Result<Vec<PathBuf>> {
    if !path.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut sheets: Vec<PathBuf> = std::fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "css"))
        .collect();
    sheets.sort();
    Ok(sheets)
}

pub(crate) fn check_built_css(root: &Path, paths: &[PathBuf]) -> Result<()> {
    let declared = declared_layers(&read(root, GLOBAL_CSS)?);
    if declared.last().map(String::as_str) != Some("sec") {
        return Err(anyhow!(
            "{GLOBAL_CSS}  its first `@layer` statement must end with `sec`"
        ));
    }
    let mut sheets = Vec::new();
    for path in paths {
        sheets.extend(built_sheets(path)?);
    }
    if sheets.is_empty() {
        return Err(anyhow!(
            "no built stylesheet at {paths:?}: run `pnpm exec vite build` first"
        ));
    }
    let mut hits = Vec::new();
    for path in sheets {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let built = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        hits.extend(built_problems(&rel, &built, &declared));
    }
    report(&hits, "built-CSS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_floors_are_read_from_the_conf() {
        let ok = serde_json::json!({"bundle": {"macOS": {"minimumSystemVersion": "14.0"},
            "linux": {"deb": {"depends": [DEB_WEBKIT]}, "rpm": {"depends": [RPM_WEBKIT, "x"]}}}});
        assert!(conf_problems(&ok).is_empty());
        let old = serde_json::json!({"bundle": {"macOS": {"minimumSystemVersion": "10.15"},
            "linux": {"deb": {"depends": [DEB_WEBKIT]}}}});
        let hits = conf_problems(&old);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(
            hits[0].contains("10.15") && hits[1].contains("rpm"),
            "{hits:?}"
        );
    }

    #[test]
    fn the_vite_targets_must_be_the_floor() {
        let ok =
            r#"target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "safari16.4","#;
        assert!(vite_problems(ok).is_empty());
        let low = ok.replace("chrome111", "chrome105");
        assert!(vite_problems(&low)[0].contains("chrome105"));
        assert_eq!(
            vite_targets(r#""windows" "esbuild" "es2022""#),
            vec!["es2022"]
        );
    }

    #[test]
    fn the_release_doc_names_every_floor() {
        let doc = format!("{MACOS_FLOOR} {DEB_WEBKIT} {RPM_WEBKIT} chrome111 safari16.4");
        assert!(doc_problems(&doc).is_empty());
        assert_eq!(
            doc_problems(&doc.replace("chrome111", "chrome105")).len(),
            1
        );
    }

    #[test]
    fn the_built_css_keeps_the_declared_layers_in_order() {
        let declared = declared_layers("/* a; */ @layer theme, base, a11y, sec;\n@import \"x\";");
        assert_eq!(declared, vec!["theme", "base", "a11y", "sec"]);
        let built = "@layer properties{a{--x:1}}@layer theme{:root{--c:red}}@layer base{b{margin:0}}@layer a11y{c{color:red}}@layer sec{d{color:red}}:root{--z:1}@media (x){@layer base{e{margin:0}}}";
        assert!(built_problems("o.css", built, &declared).is_empty());
        let swapped = built.replace(
            "@layer a11y{c{color:red}}@layer sec{d{color:red}}",
            "@layer sec{d{color:red}}@layer a11y{c{color:red}}",
        );
        assert!(built_problems("o.css", &swapped, &declared)[0].contains("opens the layers"));
        let dropped = built.replace("@layer sec{d{color:red}}", "");
        assert_eq!(built_problems("o.css", &dropped, &declared).len(), 1);
        let unlayered = format!("{built}.f{{color:red}}");
        assert!(built_problems("o.css", &unlayered, &declared)[0].contains("outside every layer"));
    }
}

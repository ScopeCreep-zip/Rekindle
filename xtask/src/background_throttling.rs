//! `check-background-throttling` (plan A1.5, finding E7.14 F5): every
//! window throttles in the background and is never suspended.
//!
//! Without a policy, macOS WKWebView's default inactive scheduling fully
//! suspends a hidden or covered webview's tasks (~5 min after it is
//! hidden, tauri-utils `BackgroundThrottlingPolicy`), so a buddy list in
//! the tray or a covered chat window stops handling events, timers and
//! badges until shown. Tauri's `Throttle` ("limits processing, but does
//! not fully suspend tasks") matches Signal and Element Desktop, which
//! keep Electron's default `backgroundThrottling: true`. Tauri's builder
//! has no read-back for the policy, so this gate reads the source:
//!
//! - every window builder constructor under `src-tauri/src`
//!   (`WebviewWindowBuilder`, `WebviewBuilder` or `WindowBuilder`, `::new`
//!   or `::from_config`) is called only inside `windows::base_builder`;
//! - `base_builder` calls `.background_throttling(…::Throttle)`;
//! - every window `src-tauri/tauri.conf.json` declares sets
//!   `"backgroundThrottling": "throttle"`.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use syn::visit::Visit;

use crate::walk_source_files;

const WINDOWS_RS: &str = "src-tauri/src/windows/mod.rs";
const BASE_BUILDER: &str = "base_builder";
const CONF: &str = "src-tauri/tauri.conf.json";
const BUILDERS: &[&str] = &["WebviewWindowBuilder", "WebviewBuilder", "WindowBuilder"];
const CONSTRUCTORS: &[&str] = &["new", "from_config"];

#[derive(Default)]
struct Finder {
    fn_stack: Vec<String>,
    /// Enclosing fn of each builder constructor call.
    constructors: Vec<(String, String)>,
    /// `base_builder` calls `.background_throttling(…::Throttle)`.
    throttles: bool,
}

impl Finder {
    fn current_fn(&self) -> String {
        self.fn_stack.last().cloned().unwrap_or_default()
    }
}

fn last_segments(path: &syn::Path) -> Option<(String, String)> {
    let mut segs = path.segments.iter().rev();
    let last = segs.next()?.ident.to_string();
    let prev = segs.next()?.ident.to_string();
    Some((prev, last))
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.fn_stack.push(f.sig.ident.to_string());
        syn::visit::visit_item_fn(self, f);
        self.fn_stack.pop();
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.fn_stack.push(f.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, f);
        self.fn_stack.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*call.func {
            if let Some((ty, ctor)) = last_segments(&p.path) {
                if BUILDERS.contains(&ty.as_str()) && CONSTRUCTORS.contains(&ctor.as_str()) {
                    self.constructors
                        .push((format!("{ty}::{ctor}"), self.current_fn()));
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "background_throttling" && self.current_fn() == BASE_BUILDER {
            let throttle = call.args.iter().any(|a| {
                matches!(a, syn::Expr::Path(p)
                    if p.path.segments.last().is_some_and(|s| s.ident == "Throttle"))
            });
            self.throttles |= throttle;
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// The problems of one Rust source file at `rel`.
fn source_problems(rel: &str, src: &str) -> Result<Vec<String>> {
    let file = syn::parse_file(src).with_context(|| format!("parse {rel}"))?;
    let mut finder = Finder::default();
    finder.visit_file(&file);
    let mut out = Vec::new();
    for (ctor, in_fn) in &finder.constructors {
        if rel != WINDOWS_RS || in_fn != BASE_BUILDER {
            out.push(format!(
                "{rel}  `{ctor}` in `{in_fn}`: every window is built from `{WINDOWS_RS}` \
                 `{BASE_BUILDER}`, which sets its background throttling"
            ));
        }
    }
    if rel == WINDOWS_RS && !finder.throttles {
        out.push(format!(
            "{rel}  `{BASE_BUILDER}` does not call \
             `.background_throttling(BackgroundThrottlingPolicy::Throttle)`"
        ));
    }
    Ok(out)
}

/// The problems of the windows `tauri.conf.json` declares.
fn conf_problems(conf: &serde_json::Value) -> Vec<String> {
    let windows = conf
        .pointer("/app/windows")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    windows
        .iter()
        .enumerate()
        .filter(|(_, w)| w.get("backgroundThrottling").and_then(|v| v.as_str()) != Some("throttle"))
        .map(|(i, w)| {
            let label = w.get("label").and_then(|v| v.as_str()).unwrap_or("main");
            format!(
                "{CONF}  app.windows[{i}] (`{label}`) lacks \"backgroundThrottling\": \"throttle\""
            )
        })
        .collect()
}

pub(crate) fn check_background_throttling(root: &Path) -> Result<()> {
    let mut hits = Vec::new();
    let windows_rs = root.join(WINDOWS_RS);
    if !windows_rs.exists() {
        hits.push(format!("{WINDOWS_RS} is missing"));
    }
    for path in walk_source_files(&root.join("src-tauri/src"), &["rs"])? {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(&path).with_context(|| format!("read {rel}"))?;
        hits.extend(source_problems(&rel, &src)?);
    }
    let conf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(CONF)).with_context(|| format!("read {CONF}"))?,
    )
    .with_context(|| format!("parse {CONF}"))?;
    hits.extend(conf_problems(&conf));
    if hits.is_empty() {
        println!("check-background-throttling: every window throttles, none suspends");
        return Ok(());
    }
    for h in &hits {
        eprintln!("{h}");
    }
    Err(anyhow!(
        "check-background-throttling: {} problem(s)",
        hits.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r"
        fn base_builder() {
            let b = WebviewWindowBuilder::new(app, label, url)
                .background_throttling(BackgroundThrottlingPolicy::Throttle);
        }
        fn open() { base_builder(); }
    ";

    #[test]
    fn base_builder_with_throttle_passes() {
        assert!(source_problems(WINDOWS_RS, GOOD).unwrap().is_empty());
    }

    #[test]
    fn missing_or_other_policy_fails() {
        let none = GOOD.replace(
            ".background_throttling(BackgroundThrottlingPolicy::Throttle)",
            "",
        );
        assert_eq!(source_problems(WINDOWS_RS, &none).unwrap().len(), 1);
        let suspend = GOOD.replace("::Throttle", "::Suspend");
        assert_eq!(source_problems(WINDOWS_RS, &suspend).unwrap().len(), 1);
    }

    #[test]
    fn builder_outside_base_builder_fails() {
        let other = "fn open() { let b = tauri::WebviewWindowBuilder::from_config(app, &c); }";
        assert_eq!(
            source_problems("src-tauri/src/x.rs", other).unwrap().len(),
            1
        );
        let in_mod = format!("{GOOD} fn rogue() {{ WindowBuilder::new(app, l); }}");
        assert_eq!(source_problems(WINDOWS_RS, &in_mod).unwrap().len(), 1);
    }

    #[test]
    fn conf_windows_need_throttle() {
        let empty = serde_json::json!({"app": {"windows": []}});
        assert!(conf_problems(&empty).is_empty());
        let bad = serde_json::json!({"app": {"windows": [{"label": "x"}]}});
        assert_eq!(conf_problems(&bad).len(), 1);
        let good = serde_json::json!({"app": {"windows": [
            {"label": "x", "backgroundThrottling": "throttle"}
        ]}});
        assert!(conf_problems(&good).is_empty());
    }
}

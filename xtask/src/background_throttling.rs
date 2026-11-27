//! `check-background-throttling` (plan A1.5, finding E7.14 F5): no window
//! is ever suspended in the background; a window that can carry live media
//! (call, chat, community) is not throttled either (lead ruling, E7.14
//! key-status: a hidden community window's timers ran at ~1 Hz with gaps
//! up to 69 s under `Throttle`).
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
//! - `base_builder` calls `.background_throttling(…::Throttle)`, or
//!   `.background_throttling(background_policy(kind))` where
//!   `windows::background_policy` names only `Throttle` and `Disabled`,
//!   and `Throttle` at least once (the text windows);
//! - every window `src-tauri/tauri.conf.json` declares sets
//!   `"backgroundThrottling": "throttle"`.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use syn::visit::Visit;

use crate::walk_source_files;

const WINDOWS_RS: &str = "src-tauri/src/windows/mod.rs";
const BASE_BUILDER: &str = "base_builder";
const POLICY_FN: &str = "background_policy";
/// The policies `background_policy` may name: never `Suspend`.
const ALLOWED: &[&str] = &["Throttle", "Disabled"];
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
    /// `base_builder` calls `.background_throttling(background_policy(…))`.
    by_kind: bool,
    /// `background_policy` is defined here.
    policy_fn: bool,
    /// The `BackgroundThrottlingPolicy` variants `background_policy` names.
    policy_variants: Vec<String>,
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
        self.policy_fn |= f.sig.ident == POLICY_FN;
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
            let by_kind = call.args.iter().any(|a| {
                matches!(a, syn::Expr::Call(c)
                    if matches!(&*c.func, syn::Expr::Path(p)
                        if p.path.segments.last().is_some_and(|s| s.ident == POLICY_FN)))
            });
            self.by_kind |= by_kind;
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.current_fn() == POLICY_FN {
            if let Some((ty, variant)) = last_segments(path) {
                if ty == "BackgroundThrottlingPolicy" {
                    self.policy_variants.push(variant);
                }
            }
        }
        syn::visit::visit_path(self, path);
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
    if rel == WINDOWS_RS && !finder.throttles && !finder.by_kind {
        out.push(format!(
            "{rel}  `{BASE_BUILDER}` does not call \
             `.background_throttling(BackgroundThrottlingPolicy::Throttle)` \
             or `.background_throttling({POLICY_FN}(kind))`"
        ));
    }
    if rel == WINDOWS_RS && finder.by_kind {
        out.extend(policy_problems(rel, &finder));
    }
    Ok(out)
}

/// The problems of `background_policy`, when `base_builder` takes its
/// policy from it: defined, never `Suspend` (nor anything but `Throttle`
/// and `Disabled`), and `Throttle` for some kind.
fn policy_problems(rel: &str, finder: &Finder) -> Vec<String> {
    if !finder.policy_fn {
        return vec![format!(
            "{rel}  `{BASE_BUILDER}` calls `{POLICY_FN}`, which is not defined here"
        )];
    }
    let mut out: Vec<String> = finder
        .policy_variants
        .iter()
        .filter(|v| !ALLOWED.contains(&v.as_str()))
        .map(|v| format!("{rel}  `{POLICY_FN}` names `BackgroundThrottlingPolicy::{v}`: only `Throttle` and `Disabled`"))
        .collect();
    if !finder.policy_variants.iter().any(|v| v == "Throttle") {
        out.push(format!(
            "{rel}  `{POLICY_FN}` throttles no window: the text windows throttle"
        ));
    }
    out
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
        println!(
            "check-background-throttling: no window suspends; live-media windows are not throttled"
        );
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
#[path = "background_throttling_tests.rs"]
mod tests;

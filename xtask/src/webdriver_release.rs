//! `check-webdriver-release`: the WebDriver walk's driver stays out of every
//! release build (plan E7.13 "Driver").
//!
//! The vendored `tauri-plugin-wdio-webdriver` serves a loopback remote
//! control of every window, `POST /wdio/eval` among it. A release build that
//! carried it would ship that control, so this gate requires:
//! - its `src/lib.rs` has `#[cfg(not(debug_assertions))] compile_error!`,
//!   and `src-tauri/src/lib.rs` has
//!   `#[cfg(all(feature = "webdriver-test", not(debug_assertions)))]
//!   compile_error!`, so a release profile compiles neither;
//! - its manifest says `publish = false`, it is not a workspace default
//!   member and the root `[workspace.dependencies]` does not name it;
//! - `rekindle-desktop` names it only as an `optional = true` dependency
//!   behind the `webdriver-test` feature, which is not a default, which the
//!   resolve does not enable and which no package's features turn on;
//! - in the graph `cargo metadata` resolves, nothing depends on it and no
//!   release binary's build graph reaches it;
//! - no release binary already built under `target/release` holds
//!   `/wdio/eval` (this gate's own binary aside: the route is its needle).

use std::path::Path;

use anyhow::{anyhow, Context, Result};

const CRATE: &str = "tauri-plugin-wdio-webdriver";
const CRATE_DIR: &str = "crates/vendor/tauri-plugin-wdio-webdriver";
const HOST: &str = "rekindle-desktop";
const FEATURE: &str = "webdriver-test";
const EVAL_ROUTE: &[u8] = b"/wdio/eval";

/// Whether `lib` has `#[cfg(not(debug_assertions))] compile_error!`.
fn has_release_guard(lib: &str) -> bool {
    let Ok(file) = syn::parse_file(lib) else {
        return false;
    };
    file.items.iter().any(|item| {
        let syn::Item::Macro(m) = item else {
            return false;
        };
        let is_compile_error = m.mac.path.is_ident("compile_error");
        let gated = m.attrs.iter().any(|a| {
            a.path().is_ident("cfg")
                && a.parse_args::<syn::Meta>().is_ok_and(|meta| {
                    let syn::Meta::List(not) = meta else {
                        return false;
                    };
                    not.path.is_ident("not") && not.tokens.to_string() == "debug_assertions"
                })
        });
        is_compile_error && gated
    })
}

/// Whether `lib` has `#[cfg(all(feature = "webdriver-test",
/// not(debug_assertions)))] compile_error!`.
fn has_feature_release_guard(lib: &str) -> bool {
    let Ok(file) = syn::parse_file(lib) else {
        return false;
    };
    let wanted = format!("all (feature = \"{FEATURE}\" , not (debug_assertions))");
    file.items.iter().any(|item| {
        let syn::Item::Macro(m) = item else {
            return false;
        };
        m.mac.path.is_ident("compile_error")
            && m.attrs.iter().any(|a| {
                a.path().is_ident("cfg")
                    && a.parse_args::<syn::Meta>().is_ok_and(|meta| {
                        quote::ToTokens::to_token_stream(&meta).to_string() == wanted
                    })
            })
    })
}

/// What lets the driver in through `rekindle-desktop`'s features.
fn feature_problems(
    meta: &crate::cargo_meta::Metadata,
    host_manifest: &toml_edit::DocumentMut,
) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(host) = meta.package(HOST) else {
        return vec![format!("{HOST} is not in the workspace")];
    };
    match host.features.get(FEATURE) {
        Some(enables) if enables.iter().any(|f| f == &format!("dep:{CRATE}")) => {}
        _ => problems.push(format!(
            "{HOST} has no {FEATURE} = [\"dep:{CRATE}\"] feature"
        )),
    }
    if host
        .features
        .get("default")
        .is_some_and(|d| d.iter().any(|f| f == FEATURE))
    {
        problems.push(format!("{FEATURE} is a default feature of {HOST}"));
    }
    if meta.resolved_features(HOST).iter().any(|f| f == FEATURE) {
        problems.push(format!("the workspace resolve enables {HOST}/{FEATURE}"));
    }
    for package in &meta.packages {
        let enables = package
            .features
            .values()
            .flatten()
            .any(|f| f.starts_with(HOST) && f.ends_with(&format!("/{FEATURE}")));
        if enables {
            problems.push(format!("{} enables {HOST}/{FEATURE}", package.name));
        }
    }
    let optional = host_manifest
        .get("dependencies")
        .and_then(|d| d.get(CRATE))
        .and_then(|d| d.get("optional"))
        .and_then(toml_edit::Item::as_bool);
    if optional != Some(true) {
        problems.push(format!(
            "src-tauri/Cargo.toml does not name {CRATE} as optional = true"
        ));
    }
    problems
}

/// What reaches the driver in the resolved graph.
fn graph_problems(meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    let mut problems: Vec<String> = meta
        .direct_dependents(CRATE)
        .into_iter()
        .map(|name| format!("{name} depends on {CRATE} in the default resolve"))
        .collect();
    for member in meta.members() {
        let ships_binary = member
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "bin"));
        if ships_binary && meta.build_graph(&member.name).contains(CRATE) {
            problems.push(format!("the {} build graph holds {CRATE}", member.name));
        }
    }
    problems
}

/// Release binaries under `target/release` that hold the eval route.
///
/// This gate's own binary is not scanned: it holds [`EVAL_ROUTE`] as the
/// needle it searches for, so a release build of it (`cargo run --release
/// -p xtask`, as a skin re-import does) would always match. It is a
/// development tool no bundle ships.
fn release_binary_problems(root: &Path, meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    let release = root.join("target").join("release");
    meta.members()
        .filter(|m| m.name != env!("CARGO_PKG_NAME"))
        .flat_map(|m| m.targets.iter())
        .filter(|t| t.kind.iter().any(|k| k == "bin"))
        .filter_map(|t| {
            let bin = release.join(&t.name);
            let bytes = std::fs::read(&bin).ok()?;
            bytes
                .windows(EVAL_ROUTE.len())
                .any(|w| w == EVAL_ROUTE)
                .then(|| format!("target/release/{} holds /wdio/eval", t.name))
        })
        .collect()
}

fn workspace_names_driver(manifest: &toml_edit::DocumentMut) -> bool {
    manifest
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(toml_edit::Item::as_table_like)
        .is_some_and(|deps| {
            deps.iter().any(|(key, dep)| {
                key == CRATE
                    || dep
                        .as_table_like()
                        .and_then(|t| t.get("package"))
                        .and_then(toml_edit::Item::as_str)
                        == Some(CRATE)
            })
        })
}

fn read_manifest(path: &Path) -> Result<toml_edit::DocumentMut> {
    std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))
}

pub(crate) fn check_webdriver_release(root: &Path) -> Result<()> {
    let mut problems = Vec::new();
    let dir = root.join(CRATE_DIR);
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap_or_default();
    if !has_release_guard(&lib) {
        problems.push(format!(
            "{CRATE_DIR}/src/lib.rs lacks #[cfg(not(debug_assertions))] compile_error!"
        ));
    }
    let host_lib = std::fs::read_to_string(root.join("src-tauri/src/lib.rs")).unwrap_or_default();
    if !has_feature_release_guard(&host_lib) {
        problems.push(format!(
            "src-tauri/src/lib.rs lacks #[cfg(all(feature = \"{FEATURE}\", not(debug_assertions)))] compile_error!"
        ));
    }
    let own = read_manifest(&dir.join("Cargo.toml"))?;
    let publish = own.get("package").and_then(|p| p.get("publish"));
    if publish.and_then(toml_edit::Item::as_bool) != Some(false) {
        problems.push(format!("{CRATE_DIR}/Cargo.toml lacks publish = false"));
    }
    let workspace = read_manifest(&root.join("Cargo.toml"))?;
    let default_members = workspace
        .get("workspace")
        .and_then(|w| w.get("default-members"))
        .and_then(toml_edit::Item::as_array);
    if default_members.is_some_and(|m| m.iter().any(|v| v.as_str() == Some(CRATE_DIR))) {
        problems.push(format!("{CRATE} is a workspace default member"));
    }
    if workspace_names_driver(&workspace) {
        problems.push(format!("the root [workspace.dependencies] names {CRATE}"));
    }
    let meta = crate::cargo_meta::Metadata::load(root)?;
    let host_manifest = read_manifest(&root.join("src-tauri/Cargo.toml"))?;
    problems.extend(graph_problems(&meta));
    problems.extend(feature_problems(&meta, &host_manifest));
    problems.extend(release_binary_problems(root, &meta));
    if problems.is_empty() {
        return Ok(());
    }
    for p in &problems {
        eprintln!("    {p}");
    }
    Err(anyhow!(
        "the WebDriver driver can reach a release build ({} problem(s))",
        problems.len()
    ))
}

#[cfg(test)]
#[path = "webdriver_release_tests.rs"]
mod tests;

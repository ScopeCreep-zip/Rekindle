//! `check-e2e-server-release`: the E2E bridge stays out of every release
//! build (research batch 8 AL, F1).
//!
//! `rekindle-e2e-server` drives a real Host over loopback HTTP for the
//! Playwright suite. A release build that carried it would ship a local
//! remote control of the user's identity, so this gate requires:
//! - its `src/lib.rs` has `#[cfg(not(debug_assertions))] compile_error!`,
//!   so a release profile cannot compile it;
//! - its manifest says `publish = false`;
//! - it is not a workspace default member (a bare `cargo build --release`
//!   at the root does not try it);
//! - nothing depends on it: in the graph `cargo metadata` resolves, no
//!   package has it as a dependency of any kind (so a renamed dependency,
//!   `bridge = { package = "rekindle-e2e-server" }`, counts), no release
//!   binary's build graph (every workspace member with a binary, the GUI
//!   `rekindle-desktop` among them) reaches it, and the root
//!   `[workspace.dependencies]` does not name it;
//! - its binary builds only with the `e2e-bridge` feature
//!   (`required-features`, backed by a `compile_error!` in the binary), the
//!   feature is not a default, the resolve does not enable it, and no
//!   package's features turn it on;
//! - `src-tauri/tauri.conf.json` does not bundle it.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

const CRATE: &str = "rekindle-e2e-server";

pub(crate) fn has_release_guard(lib: &str) -> bool {
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

/// The feature only the Playwright setup enables.
const FEATURE: &str = "e2e-bridge";

/// What lets the bridge binary build without [`FEATURE`], or turns the
/// feature on outside the e2e runner.
fn feature_problems(meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(bridge) = meta.package(CRATE) else {
        return vec![format!("{CRATE} is not in the workspace")];
    };
    if !bridge.features.contains_key(FEATURE) {
        problems.push(format!("{CRATE} has no {FEATURE} feature"));
    }
    if bridge
        .features
        .get("default")
        .is_some_and(|d| d.iter().any(|f| f == FEATURE))
    {
        problems.push(format!("{FEATURE} is a default feature of {CRATE}"));
    }
    for bin in bridge
        .targets
        .iter()
        .filter(|t| t.kind.iter().any(|k| k == "bin"))
    {
        if !bin.required_features.iter().any(|f| f == FEATURE) {
            problems.push(format!(
                "a {CRATE} binary does not require the {FEATURE} feature"
            ));
        }
    }
    if meta.resolved_features(CRATE).iter().any(|f| f == FEATURE) {
        problems.push(format!("the workspace resolve enables {CRATE}/{FEATURE}"));
    }
    for package in &meta.packages {
        let enables = package
            .features
            .values()
            .flatten()
            .any(|f| f.starts_with(CRATE) && f.ends_with(&format!("/{FEATURE}")));
        if enables {
            problems.push(format!("{} enables {CRATE}/{FEATURE}", package.name));
        }
    }
    problems
}

/// The bundle entries of `conf` (`tauri.conf.json`) that ship the bridge:
/// any `bundle.externalBin` path or `bundle.resources` source or target
/// (a list, or a map of source to target) naming one of its binaries
/// `bins` (with `-` or `_`), the crate or its directory.
fn bundle_problems(conf: &serde_json::Value, bins: &[String]) -> Vec<String> {
    let mut needles: Vec<String> = vec![CRATE.to_owned(), format!("crates/{CRATE}")];
    for bin in bins {
        needles.push(bin.clone());
        needles.push(bin.replace('-', "_"));
    }
    let bundle = &conf["bundle"];
    let mut entries: Vec<&str> = Vec::new();
    for key in ["externalBin", "resources"] {
        match &bundle[key] {
            serde_json::Value::Array(items) => {
                entries.extend(items.iter().filter_map(serde_json::Value::as_str));
            }
            serde_json::Value::Object(map) => {
                for (source, target) in map {
                    entries.push(source);
                    entries.extend(target.as_str());
                }
            }
            _ => {}
        }
    }
    entries
        .into_iter()
        .filter(|entry| {
            let entry = entry.replace('\\', "/");
            needles.iter().any(|n| entry.contains(n.as_str()))
        })
        .map(|entry| format!("src-tauri/tauri.conf.json bundles the E2E bridge ({entry})"))
        .collect()
}

/// A `[workspace.dependencies]` entry naming the bridge, by key or by
/// `package =`.
fn workspace_names_bridge(manifest: &toml_edit::DocumentMut) -> bool {
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

/// What reaches the bridge in the resolved graph: each package depending
/// on it directly, and each release binary whose build graph holds it.
fn graph_problems(meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    let mut problems: Vec<String> = meta
        .direct_dependents(CRATE)
        .into_iter()
        .map(|name| format!("{name} depends on {CRATE}"))
        .collect();
    for member in meta.members() {
        let ships_binary = member
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "bin"));
        if member.name != CRATE && ships_binary && meta.build_graph(&member.name).contains(CRATE) {
            problems.push(format!("the {} build graph holds {CRATE}", member.name));
        }
    }
    problems
}

fn read_manifest(path: &Path) -> Result<toml_edit::DocumentMut> {
    std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .parse()
        .with_context(|| format!("parsing {}", path.display()))
}

pub(crate) fn check_e2e_server_release(root: &Path) -> Result<()> {
    let mut problems = Vec::new();
    let dir = root.join("crates").join(CRATE);
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap_or_default();
    if !has_release_guard(&lib) {
        problems.push(format!(
            "crates/{CRATE}/src/lib.rs lacks #[cfg(not(debug_assertions))] compile_error!"
        ));
    }
    let own = read_manifest(&dir.join("Cargo.toml"))?;
    let publish = own.get("package").and_then(|p| p.get("publish"));
    if publish.and_then(toml_edit::Item::as_bool) != Some(false) {
        problems.push(format!("crates/{CRATE}/Cargo.toml lacks publish = false"));
    }
    let meta = crate::cargo_meta::Metadata::load(root)?;
    let workspace = read_manifest(&root.join("Cargo.toml"))?;
    let default_members = workspace
        .get("workspace")
        .and_then(|w| w.get("default-members"))
        .and_then(toml_edit::Item::as_array);
    if default_members.is_some_and(|m| {
        m.iter()
            .any(|v| v.as_str() == Some(&format!("crates/{CRATE}")))
    }) {
        problems.push(format!("{CRATE} is a workspace default member"));
    }
    if workspace_names_bridge(&workspace) {
        problems.push(format!("the root [workspace.dependencies] names {CRATE}"));
    }
    problems.extend(graph_problems(&meta));
    problems.extend(feature_problems(&meta));
    let conf: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("src-tauri/tauri.conf.json"))
            .context("reading src-tauri/tauri.conf.json")?,
    )
    .context("parsing src-tauri/tauri.conf.json")?;
    let bridge = meta
        .package(CRATE)
        .map(|p| {
            p.targets
                .iter()
                .filter(|t| t.kind.iter().any(|k| k == "bin"))
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    problems.extend(bundle_problems(&conf, &bridge));
    if problems.is_empty() {
        return Ok(());
    }
    for p in &problems {
        eprintln!("    {p}");
    }
    Err(anyhow!(
        "the E2E bridge can reach a release build ({} problem(s))",
        problems.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        feature_problems, graph_problems, has_release_guard, workspace_names_bridge, CRATE,
    };

    #[test]
    fn the_release_guard_is_recognised() {
        let guarded = "#[cfg(not(debug_assertions))]\ncompile_error!(\"no\");";
        assert!(has_release_guard(guarded));
        assert!(!has_release_guard("compile_error!(\"always\");"));
        assert!(!has_release_guard("#[cfg(test)]\ncompile_error!(\"no\");"));
    }

    #[test]
    fn a_workspace_dependency_on_the_bridge_is_found_by_key_or_package() {
        let parse = |t: &str| t.parse::<toml_edit::DocumentMut>().unwrap();
        let key = parse("[workspace.dependencies]\nrekindle-e2e-server = { path = \"x\" }");
        assert!(workspace_names_bridge(&key));
        let renamed = parse(
            "[workspace.dependencies]\nbridge = { package = \"rekindle-e2e-server\", path = \"x\" }",
        );
        assert!(workspace_names_bridge(&renamed));
        let none = parse("[workspace.dependencies]\nserde = \"1\"");
        assert!(!workspace_names_bridge(&none));
    }

    #[test]
    fn a_bundled_bridge_is_found_in_external_bins_and_resources() {
        let bins = ["e2e-server".to_owned()];
        let conf = |bundle: serde_json::Value| serde_json::json!({ "bundle": bundle });
        let ext = conf(serde_json::json!({ "externalBin": ["binaries/e2e_server"] }));
        assert_eq!(super::bundle_problems(&ext, &bins).len(), 1);
        let list = conf(serde_json::json!({ "resources": ["../crates/rekindle-e2e-server/x"] }));
        assert_eq!(super::bundle_problems(&list, &bins).len(), 1);
        let map =
            conf(serde_json::json!({ "resources": { "../target/debug/e2e-server": "tools/x" } }));
        assert_eq!(super::bundle_problems(&map, &bins).len(), 1);
        let clean = conf(
            serde_json::json!({ "externalBin": ["binaries/rekindled"], "icon": ["e2e-server.png"] }),
        );
        assert!(super::bundle_problems(&clean, &bins).is_empty());
    }

    /// The real workspace: nothing reaches the bridge, and the GUI's build
    /// graph is the one a release ships.
    #[test]
    fn nothing_in_the_workspace_reaches_the_bridge() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let meta = crate::cargo_meta::Metadata::load(&root).unwrap();
        assert!(meta.build_graph("rekindle-desktop").contains("rekindle-db"));
        assert!(graph_problems(&meta).is_empty());
        assert!(
            feature_problems(&meta).is_empty(),
            "{:?}",
            feature_problems(&meta)
        );
        // The bridge's own graph does hold the GUI, so the walk sees edges.
        assert!(meta.build_graph(CRATE).contains("rekindle-desktop"));
    }
}

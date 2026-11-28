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
//! - the CP1 aids' requests, test modules and Host interception points are
//!   feature-gated, and the fleet and Host feature rejects release builds;
//! - release binaries under the workspace and configured Cargo target
//!   directories contain none of the driver or test-aid identifiers scanned
//!   by this check. The gate's own binary carries those strings for the scan.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use quote::ToTokens;
use syn::visit::{self, Visit};

pub(crate) const CRATE: &str = "tauri-plugin-wdio-webdriver";
const CRATE_DIR: &str = "crates/vendor/tauri-plugin-wdio-webdriver";
const HOST: &str = "rekindle-desktop";
const FEATURE: &str = "webdriver-test";
const EVAL_ROUTE: &[u8] = b"/wdio/eval";
/// The walk's idle override (`src-tauri/src/webdriver.rs`), compiled only
/// beside the driver.
const IDLE_ENV: &[u8] = b"REKINDLE_WDIO_IDLE_SECONDS";
/// The real-app suite's damaged-post switch (`webdriver.rs`): a release
/// binary naming it would damage a received post on request.
const DAMAGE_NEXT_POST_VAR: &[u8] = b"REKINDLE_WDIO_DAMAGE_NEXT_POST";
/// What no release binary may hold.
const NEEDLES: &[&[u8]] = &[
    EVAL_ROUTE,
    IDLE_ENV,
    DAMAGE_NEXT_POST_VAR,
    b"webdriver-post-skipping-local-permission",
    b"/wdio/hold-governance-ops",
    b"/wdio/release-governance-ops",
];
const AID_PACKAGES: [&str; 5] = [
    HOST,
    "rekindle-cli",
    "rekindle-node",
    "rekindle-ipc",
    "rekindle-types",
];

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
    for name in AID_PACKAGES {
        let Some(package) = meta.package(name) else {
            problems.push(format!("{name} is not in the workspace"));
            continue;
        };
        if !package.features.contains_key(FEATURE) {
            problems.push(format!("{name} has no {FEATURE} feature"));
        }
        if package
            .features
            .get("default")
            .is_some_and(|d| d.iter().any(|f| f == FEATURE))
        {
            problems.push(format!("{FEATURE} is a default feature of {name}"));
        }
        if meta.resolved_features(name).iter().any(|f| f == FEATURE) {
            problems.push(format!("the workspace resolve enables {name}/{FEATURE}"));
        }
        for dependent in &meta.packages {
            for (feature, enables) in &dependent.features {
                let enables_aid = enables.iter().any(|f| {
                    f.split_once('/').is_some_and(|(dep, enabled)| {
                        dep.trim_end_matches('?') == name && enabled == FEATURE
                    })
                });
                if enables_aid
                    && !(AID_PACKAGES.contains(&dependent.name.as_str()) && feature == FEATURE)
                {
                    problems.push(format!(
                        "{} feature {feature} enables {name}/{FEATURE}",
                        dependent.name
                    ));
                }
            }
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

/// Release binaries under `target/release` that hold one of [`NEEDLES`],
/// one problem per needle held.
///
/// This gate's own binary is not scanned: it holds [`NEEDLES`] as what it
/// searches for, so a release build of it (`cargo run --release
/// -p xtask`, as a skin re-import does) would always match. It is a
/// development tool no bundle ships.
fn release_binary_problems(root: &Path, meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    release_binaries_in(&root.join("target"), meta)
}

fn release_binaries_in(target: &Path, meta: &crate::cargo_meta::Metadata) -> Vec<String> {
    let release = target.join("release");
    meta.members()
        .filter(|m| m.name != env!("CARGO_PKG_NAME"))
        .flat_map(|m| m.targets.iter())
        .filter(|t| t.kind.iter().any(|k| k == "bin"))
        .flat_map(|t| {
            let bytes = std::fs::read(release.join(&t.name)).unwrap_or_default();
            NEEDLES
                .iter()
                .filter(|needle| bytes.windows(needle.len()).any(|w| w == **needle))
                .map(|needle| {
                    format!(
                        "target/release/{} holds {}",
                        t.name,
                        String::from_utf8_lossy(needle)
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Check the source boundaries even when there are no release artifacts.
/// These are the request declarations, the app and Host test modules, and
/// the two interception points in the normal send and governance paths.
fn aid_source_problems(root: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    for (path, markers) in [
        (
            "crates/rekindle-ipc/src/protocol/mod.rs",
            &[
                "WebdriverPostSkippingLocalPermission",
                "WebdriverHoldGovernanceOps",
                "WebdriverReleaseGovernanceOps",
            ][..],
        ),
        (
            "crates/rekindle-cli/src/cli/mod.rs",
            &["WebdriverPostSkippingLocalPermission"],
        ),
        ("crates/rekindle-node/src/host/mod.rs", &["webdriver"]),
        ("src-tauri/src/lib.rs", &["webdriver"]),
        (
            "crates/rekindle-node/src/host/adapters/channel/deps_impl.rs",
            &["self . skip_local_post_permission"],
        ),
        (
            "crates/rekindle-node/src/host/adapters/governance/op_store.rs",
            &["let Some (queued) = queue . as_mut ()"],
        ),
    ] {
        let src =
            std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))?;
        let file = syn::parse_file(&src).with_context(|| format!("parsing {path}"))?;
        for marker in markers {
            let mut guard = AidGuard {
                marker,
                declarations: Vec::new(),
            };
            guard.visit_file(&file);
            if guard.declarations != [true] {
                problems.push(format!(
                    "{path}: {marker} must appear once behind {FEATURE} and debug_assertions"
                ));
            }
        }
    }

    for path in [
        "crates/rekindle-cli/src/main.rs",
        "crates/rekindle-node/src/lib.rs",
        "crates/rekindle-ipc/src/lib.rs",
        "crates/rekindle-types/src/lib.rs",
    ] {
        let src =
            std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))?;
        if !has_feature_release_guard(&src) {
            problems.push(format!("{path} lacks the {FEATURE} release compile_error!"));
        }
    }
    Ok(problems)
}

struct AidGuard<'a> {
    marker: &'a str,
    declarations: Vec<bool>,
}

impl AidGuard<'_> {
    fn record(&mut self, attrs: &[syn::Attribute]) {
        self.declarations.push(attrs.iter().any(|attr| {
            attr.path().is_ident("cfg")
                && attr.parse_args::<syn::Meta>().is_ok_and(|meta| {
                    meta.to_token_stream().to_string()
                        == "all (feature = \"webdriver-test\" , debug_assertions)"
                })
        }));
    }
}

impl<'ast> Visit<'ast> for AidGuard<'_> {
    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        if variant.ident == self.marker {
            self.record(&variant.attrs);
        }
        visit::visit_variant(self, variant);
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if module.ident == self.marker {
            self.record(&module.attrs);
        }
        visit::visit_item_mod(self, module);
    }

    fn visit_expr_if(&mut self, branch: &'ast syn::ExprIf) {
        if branch
            .cond
            .to_token_stream()
            .to_string()
            .starts_with(self.marker)
        {
            self.record(&branch.attrs);
        }
        visit::visit_expr_if(self, branch);
    }
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
    problems.extend(aid_source_problems(root)?);
    problems.extend(graph_problems(&meta));
    problems.extend(feature_problems(&meta, &host_manifest));
    problems.extend(release_binary_problems(root, &meta));
    if meta.target_directory != root.join("target") {
        problems.extend(release_binaries_in(&meta.target_directory, &meta));
    }
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

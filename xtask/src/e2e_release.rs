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
//! - no other workspace manifest depends on it;
//! - `src-tauri/tauri.conf.json` does not bundle it.

use std::path::Path;

use anyhow::{anyhow, Context, Result};

const CRATE: &str = "rekindle-e2e-server";

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

fn depends_on_bridge(manifest: &toml_edit::DocumentMut) -> bool {
    let in_table = |t: Option<&toml_edit::Item>| {
        t.and_then(toml_edit::Item::as_table_like)
            .is_some_and(|t| t.contains_key(CRATE))
    };
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .any(|k| in_table(manifest.get(k)))
        || manifest
            .get("target")
            .and_then(toml_edit::Item::as_table_like)
            .is_some_and(|targets| {
                targets.iter().any(|(_, t)| {
                    ["dependencies", "dev-dependencies", "build-dependencies"]
                        .iter()
                        .any(|k| in_table(t.get(k)))
                })
            })
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
    let mut manifests = vec![root.join("src-tauri/Cargo.toml")];
    for entry in std::fs::read_dir(root.join("crates"))? {
        let path = entry?.path();
        if path.file_name().is_some_and(|n| n != CRATE) {
            manifests.push(path.join("Cargo.toml"));
        }
    }
    for path in manifests.iter().filter(|p| p.exists()) {
        if depends_on_bridge(&read_manifest(path)?) {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .display()
                .to_string();
            problems.push(format!("{rel} depends on {CRATE}"));
        }
    }
    let conf = std::fs::read_to_string(root.join("src-tauri/tauri.conf.json")).unwrap_or_default();
    if conf.contains("e2e-server") || conf.contains("e2e_server") {
        problems.push("src-tauri/tauri.conf.json bundles the e2e server".to_owned());
    }
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
    use super::{depends_on_bridge, has_release_guard};

    #[test]
    fn the_release_guard_is_recognised() {
        let guarded = "#[cfg(not(debug_assertions))]\ncompile_error!(\"no\");";
        assert!(has_release_guard(guarded));
        assert!(!has_release_guard("compile_error!(\"always\");"));
        assert!(!has_release_guard("#[cfg(test)]\ncompile_error!(\"no\");"));
    }

    #[test]
    fn a_dependency_on_the_bridge_is_found_in_any_table() {
        let plain: toml_edit::DocumentMut = "[dependencies]\nrekindle-e2e-server = \"0\""
            .parse()
            .unwrap();
        assert!(depends_on_bridge(&plain));
        let target: toml_edit::DocumentMut =
            "[target.'cfg(unix)'.dev-dependencies]\nrekindle-e2e-server = \"0\""
                .parse()
                .unwrap();
        assert!(depends_on_bridge(&target));
        let none: toml_edit::DocumentMut = "[dependencies]\nserde = \"1\"".parse().unwrap();
        assert!(!depends_on_bridge(&none));
    }
}

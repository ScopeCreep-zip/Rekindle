//! `cargo xtask gen-skin-registry` and `check-skin-registry` (plan E7.7.1,
//! the closed token registry).
//!
//! The generator writes the registry's artefacts from
//! `rekindle_skin::generate`: the app's token CSS and TS, the two TUI
//! palettes and the JSON Schema of a token file. Each carries a "generated;
//! do not edit" header in its own comment syntax (the JSON Schema in
//! `$comment`). The check fails when any file on disk differs from what the
//! generator would write now, and when a base's defaults break one of the
//! registry's own contrast constraints.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use rekindle_skin::generate::{json_schema, tokens_css, tokens_ts, tui_palette};
use rekindle_skin::registry::{constraint_failures, Base, ENTRIES};

/// The generated stylesheet directory: its literals and repeats are the
/// registry's, so the theme-literal and duplication checks skip it and this
/// staleness check guards it.
pub(crate) const GENERATED_STYLES: &str = "src/styles/generated/";

/// Every generated file, relative to the workspace root, with its content.
fn artefacts() -> [(&'static str, String); 5] {
    [
        ("src/styles/generated/tokens.css", tokens_css()),
        ("src/styles/generated/tokens.ts", tokens_ts()),
        (
            "crates/rekindle-tui/themes/rekindle-dark.toml",
            tui_palette(Base::Dark),
        ),
        (
            "crates/rekindle-tui/themes/rekindle-light.toml",
            tui_palette(Base::Light),
        ),
        (
            "crates/rekindle-skin/schema/tokens.schema.json",
            json_schema(),
        ),
    ]
}

/// Writes every artefact under `root`.
pub fn gen_skin_registry(root: &Path) -> Result<()> {
    for (rel, content) in artefacts() {
        let path = root.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {rel}"))?;
        println!("    wrote {rel}");
    }
    Ok(())
}

/// The stale or missing artefacts under `root`, and every broken
/// constraint of the registry's defaults, one line each.
fn problems(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (rel, content) in artefacts() {
        match std::fs::read_to_string(root.join(rel)) {
            Ok(on_disk) if on_disk == content => {}
            Ok(_) => out.push(format!("{rel}: stale")),
            Err(e) => out.push(format!("{rel}: {e}")),
        }
    }
    for base in Base::ALL {
        for failure in constraint_failures(ENTRIES, base) {
            out.push(format!("registry defaults on {}: {failure}", base.key()));
        }
    }
    out
}

/// Fails when a generated file is stale or missing, or a base's defaults
/// break a registry constraint.
pub fn check_skin_registry(root: &Path) -> Result<()> {
    let problems = problems(root);
    if problems.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "{}\n    Change `rekindle-skin::registry`, never a generated file, then run \
         `cargo xtask gen-skin-registry`.",
        problems.join("\n    ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edited_generated_file_fails_until_regenerated() {
        let dir = std::env::temp_dir().join(format!("xtask-skin-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        gen_skin_registry(&dir).unwrap();
        check_skin_registry(&dir).unwrap();

        for (rel, _) in artefacts() {
            let path = dir.join(rel);
            let mut text = std::fs::read_to_string(&path).unwrap();
            text.push_str("\n/* hand edit */\n");
            std::fs::write(&path, text).unwrap();
            let err = check_skin_registry(&dir).unwrap_err().to_string();
            assert!(err.contains(&format!("{rel}: stale")), "{err}");

            gen_skin_registry(&dir).unwrap();
            check_skin_registry(&dir).unwrap();
        }

        std::fs::remove_file(dir.join(artefacts()[0].0)).unwrap();
        assert!(check_skin_registry(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_artefact_carries_the_do_not_edit_header() {
        for (rel, content) in artefacts()
            .into_iter()
            .filter(|(rel, _)| Path::new(rel).extension().is_none_or(|e| e != "json"))
        {
            let head = content.lines().next().unwrap_or_default();
            assert!(
                head.contains("cargo xtask gen-skin-registry") && head.contains("Do not edit"),
                "{rel}: {head}"
            );
        }
        let schema: serde_json::Value = serde_json::from_str(&json_schema()).unwrap();
        assert!(schema["$comment"]
            .as_str()
            .is_some_and(|c| c.contains("Do not edit")));
    }

    #[test]
    fn the_committed_artefacts_are_current() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        check_skin_registry(root).unwrap();
    }
}

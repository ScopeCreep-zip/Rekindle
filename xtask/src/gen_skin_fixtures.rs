//! `cargo xtask gen-skin-fixtures` and `check-skin-fixtures` (plan E7.3
//! "Gate": axe "over every window under every built-in skin").
//!
//! The mocked-IPC suites cannot load a `skin://` stylesheet, so the
//! generator writes what the Host would serve, from `rekindle-skin`:
//!
//! * `e2e/fixtures/generated/skins/<id>.css`: each built-in pack's
//!   stylesheet ([`rekindle_skin::serve::stylesheet`]) and `skins.json`,
//!   each pack's id and the base its applied variant names, so the axe
//!   suite applies every built-in the way `useSkin` does;
//! * `e2e/fixtures/generated/role-tags.json`: the Host's role-tag colours
//!   ([`rekindle_skin::role_tag::colours`]) for the role colours the
//!   fixtures use, so a mocked role carries what the Host would send.
//!
//! The check fails when a file on disk differs from what the generator
//! writes now.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use rekindle_skin::builtin::PACKS;
use rekindle_skin::role_tag;
use rekindle_skin::serve;
use serde_json::{json, Map, Value};

/// Where the fixtures go, relative to the workspace root.
const DIR: &str = "e2e/fixtures/generated";

/// The role colours `e2e/fixtures/*.ts` give their roles.
const ROLE_COLOURS: [u32; 4] = [0x0034_98db, 0x00e7_4c3c, 0x002e_cc71, 0x00f1_c40f];

/// The address a fixture stylesheet's images resolve under: never served
/// in a mocked suite, so the images stay absent and only colours apply.
const ADDRESS: &str = "fixture";

fn pretty(v: &Value) -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(v)?))
}

fn role_tags() -> Result<String> {
    let mut out = Map::new();
    for c in ROLE_COLOURS {
        let t = role_tag::colours(c, None);
        out.insert(
            format!("{c:06x}"),
            json!({ "dark": t.dark, "light": t.light }),
        );
    }
    pretty(&Value::Object(out))
}

/// Every fixture file, relative to the workspace root, with its content.
fn artefacts() -> Result<Vec<(String, String)>> {
    let mut out = vec![(format!("{DIR}/role-tags.json"), role_tags()?)];
    let mut skins = Vec::new();
    for pack in &PACKS {
        let sheet = serve::stylesheet(pack.bytes, ADDRESS)
            .map_err(|e| anyhow!("builtin:{}: {e:?}", pack.id))?;
        let base = serde_json::to_value(sheet.base)?;
        skins.push(json!({ "id": pack.id, "base": base }));
        out.push((format!("{DIR}/skins/{}.css", pack.id), sheet.css));
    }
    out.push((
        format!("{DIR}/skins/skins.json"),
        pretty(&Value::Array(skins))?,
    ));
    Ok(out)
}

/// Writes every fixture under `root`.
pub fn gen_skin_fixtures(root: &Path) -> Result<()> {
    for (rel, content) in artefacts()? {
        let path = root.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, content).with_context(|| format!("writing {rel}"))?;
        println!("    wrote {rel}");
    }
    Ok(())
}

/// Fails when a fixture is stale or missing.
pub fn check_skin_fixtures(root: &Path) -> Result<()> {
    let mut problems = Vec::new();
    for (rel, content) in artefacts()? {
        match std::fs::read_to_string(root.join(&rel)) {
            Ok(on_disk) if on_disk == content => {}
            Ok(_) => problems.push(format!("{rel}: stale")),
            Err(e) => problems.push(format!("{rel}: {e}")),
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "{}\n    Run `cargo xtask gen-skin-fixtures`.",
        problems.join("\n    ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_check_names_a_stale_fixture_and_passes_once_written() {
        let dir = std::env::temp_dir().join(format!("skin-fixtures-{}", std::process::id()));
        assert!(check_skin_fixtures(&dir).is_err());
        gen_skin_fixtures(&dir).unwrap();
        check_skin_fixtures(&dir).unwrap();
        std::fs::write(dir.join(DIR).join("role-tags.json"), "{}\n").unwrap();
        let err = check_skin_fixtures(&dir).unwrap_err().to_string();
        assert!(err.contains("role-tags.json: stale"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

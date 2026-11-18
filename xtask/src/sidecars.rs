//! `stage-sidecar` and `check-sidecars` (plan E7.7.1 "Helper process", F3
//! "Skin helper"): `rekindle-skin-helper` ships beside the GUI in every
//! Tauri bundle.
//!
//! The Host starts the helper from its own executable's directory
//! (`rekindle_skin::helper::HelperOpts::beside_current_exe`, called by
//! `rekindle-node` `host/skin/mod.rs`). Tauri 2 bundles a sidecar listed in
//! `bundle.externalBin`: the file staged for it is
//! `<entry>-<target triple>[.exe]` (tauri-utils 2.10.1 `resources.rs`
//! `external_binaries`), and the bundler copies it beside the main binary
//! with `-<target triple>` removed (tauri-bundler 2.12.1 `settings.rs`
//! `Settings::copy_binaries`): `Contents/MacOS` in the `.app`
//! (`macos/app.rs`), `usr/bin` in the `.deb` and the AppImage built from it
//! (`linux/debian.rs`), and `$INSTDIR` beside the main exe in NSIS
//! (`windows/nsis/installer.nsi`, `File /a "/oname={{this}}"` after
//! `SetOutPath $INSTDIR`). So the installed name is
//! `rekindle-skin-helper[.exe]` beside the app, where the Host looks.
//!
//! The entry lives in the release overlay [`RELEASE_CONF`], not in
//! `tauri.conf.json`: `tauri-build` copies every `externalBin` file on each
//! cargo build of the GUI (tauri-build 2.7.1 `lib.rs` `copy_binaries`) and
//! fails when one is missing (`ResourcePathNotFound`), so a base entry
//! would break `cargo check -p rekindle-desktop` until something staged the
//! helper. The overlay's `beforeBuildCommand` stages it ([`stage_sidecar`],
//! reading the `TAURI_ENV_TARGET_TRIPLE` and `TAURI_ENV_DEBUG` the Tauri CLI
//! sets for that hook), and every release entry point passes the overlay:
//! `pnpm bundle`, the release workflow's `tauri-action` step, the release
//! doc and `scripts/build.sh`. `check-sidecars` fails when any link of that
//! chain is missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use rekindle_skin::helper::HELPER_NAME;
use serde_json::Value;

pub(crate) const RELEASE_CONF: &str = "src-tauri/tauri.release.conf.json";
pub(crate) const BASE_CONF: &str = "src-tauri/tauri.conf.json";
const RELEASE_YML: &str = ".github/workflows/release.yml";
const RELEASE_DOC: &str = "docs/contributor/release-process.md";
const BUILD_SH: &str = "scripts/build.sh";
const PACKAGE_JSON: &str = "package.json";
/// Where `externalBin` entries are staged, relative to `src-tauri`.
const BINARIES_DIR: &str = "binaries";
/// The hook that stages the helper before the GUI builds.
const STAGE_COMMAND: &str = "cargo xtask stage-sidecar";
/// The flag that merges [`RELEASE_CONF`] into a `tauri build`.
const CONFIG_FLAG: &str = "--config src-tauri/tauri.release.conf.json";
/// The release build every human-run entry point uses.
const BUNDLE_COMMAND: &str = "pnpm bundle";
/// The bare build, which would bundle without the overlay.
const BARE_BUILD: &str = "pnpm tauri build";
/// The targets the release workflow builds (`release.yml` matrix).
const RELEASE_TRIPLES: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];

/// `.exe` for a Windows triple: tauri-utils `Target::from_triple` calls a
/// triple Windows when it contains `windows`, and Rust's `EXE_SUFFIX` is
/// `.exe` exactly for `target_os = "windows"`.
fn exe_suffix(triple: &str) -> &'static str {
    if triple.contains("windows") {
        ".exe"
    } else {
        ""
    }
}

/// The file Tauri expects for `name` on `triple` (`external_binaries`).
pub(crate) fn staged_name(name: &str, triple: &str) -> String {
    format!("{name}-{triple}{}", exe_suffix(triple))
}

/// The name the bundler installs a staged file under (`copy_binaries`).
pub(crate) fn installed_name(staged: &str, triple: &str) -> String {
    staged.replace(&format!("-{triple}"), "")
}

/// The name `beside_current_exe` looks for on `triple`.
fn lookup_name(triple: &str) -> String {
    format!("{HELPER_NAME}{}", exe_suffix(triple))
}

fn str_at<'a>(conf: &'a Value, pointer: &str) -> Option<&'a str> {
    conf.pointer(pointer).and_then(Value::as_str)
}

/// What keeps the helper out of a bundle, or out of the Host's sight, in
/// the two Tauri configs.
pub(crate) fn conf_problems(base: &Value, release: &Value) -> Vec<String> {
    let mut hits = Vec::new();
    if base.pointer("/bundle/externalBin").is_some() {
        hits.push(format!(
            "{BASE_CONF}  declares bundle.externalBin: tauri-build then needs the staged files on every cargo build; declare sidecars in {RELEASE_CONF}"
        ));
    }
    let entry = format!("{BINARIES_DIR}/{HELPER_NAME}");
    let entries: Vec<&str> = release
        .pointer("/bundle/externalBin")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !entries.contains(&entry.as_str()) {
        hits.push(format!(
            "{RELEASE_CONF}  bundle.externalBin lacks \"{entry}\""
        ));
    }
    for e in entries.iter().filter(|e| e.contains(HELPER_NAME)) {
        let file = Path::new(e)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or_default();
        for triple in RELEASE_TRIPLES {
            let installed = installed_name(&staged_name(file, triple), triple);
            if installed != lookup_name(triple) {
                hits.push(format!(
                    "{RELEASE_CONF}  \"{e}\" installs as {installed} on {triple}; the Host looks for {}",
                    lookup_name(triple)
                ));
            }
        }
    }
    let base_cmd = str_at(base, "/build/beforeBuildCommand").unwrap_or_default();
    let release_cmd = str_at(release, "/build/beforeBuildCommand").unwrap_or_default();
    if !release_cmd.starts_with(base_cmd) || !release_cmd.contains(STAGE_COMMAND) {
        hits.push(format!(
            "{RELEASE_CONF}  build.beforeBuildCommand must run {BASE_CONF}'s \"{base_cmd}\" and then `{STAGE_COMMAND}` (it is \"{release_cmd}\")"
        ));
    }
    hits
}

/// The `args:` of the `tauri-apps/tauri-action` step in `release_yml`.
fn action_args(release_yml: &str) -> Option<&str> {
    let mut lines = release_yml.lines();
    lines.find(|l| l.trim_start().starts_with("uses: tauri-apps/tauri-action"))?;
    lines
        .take_while(|l| !l.trim_start().starts_with("- name:"))
        .find_map(|l| l.trim_start().strip_prefix("args:"))
}

/// The release entry points that would bundle without [`RELEASE_CONF`].
pub(crate) fn entry_problems(
    release_yml: &str,
    package_json: &Value,
    release_doc: &str,
    build_sh: &str,
) -> Vec<String> {
    let mut hits = Vec::new();
    if !action_args(release_yml).is_some_and(|a| a.contains(CONFIG_FLAG)) {
        hits.push(format!(
            "{RELEASE_YML}  the tauri-action step's args lack `{CONFIG_FLAG}`"
        ));
    }
    let want = format!("tauri build {CONFIG_FLAG}");
    if str_at(package_json, "/scripts/bundle") != Some(want.as_str()) {
        hits.push(format!("{PACKAGE_JSON}  scripts.bundle must be \"{want}\""));
    }
    for (file, text) in [(RELEASE_DOC, release_doc), (BUILD_SH, build_sh)] {
        if !text.contains(BUNDLE_COMMAND) || text.contains(BARE_BUILD) {
            hits.push(format!(
                "{file}  must build with `{BUNDLE_COMMAND}`, never `{BARE_BUILD}`"
            ));
        }
    }
    hits
}

fn read(root: &Path, rel: &str) -> Result<String> {
    std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
}

fn json(root: &Path, rel: &str) -> Result<Value> {
    serde_json::from_str(&read(root, rel)?).with_context(|| format!("parsing {rel}"))
}

pub(crate) fn check_sidecars(root: &Path) -> Result<()> {
    let mut hits = conf_problems(&json(root, BASE_CONF)?, &json(root, RELEASE_CONF)?);
    hits.extend(entry_problems(
        &read(root, RELEASE_YML)?,
        &json(root, PACKAGE_JSON)?,
        &read(root, RELEASE_DOC)?,
        &read(root, BUILD_SH)?,
    ));
    if hits.is_empty() {
        return Ok(());
    }
    for h in &hits {
        eprintln!("    {h}");
    }
    Err(anyhow!(
        "{HELPER_NAME} would not ship beside the bundled app ({} problem(s))",
        hits.len()
    ))
}

fn target_directory(root: &Path, cargo: &str) -> Result<PathBuf> {
    let out = Command::new(cargo)
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        return Err(anyhow!("cargo metadata failed ({})", out.status));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).context("parsing cargo metadata")?;
    meta["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("cargo metadata has no target_directory"))
}

/// Builds the helper for `target` (or the triple the Tauri CLI names in
/// `TAURI_ENV_TARGET_TRIPLE`) and stages it as Tauri's `externalBin`
/// expects, under `src-tauri/binaries/`. A `tauri build --debug`
/// (`TAURI_ENV_DEBUG=true`) stages the debug profile.
pub(crate) fn stage_sidecar(root: &Path, target: Option<&str>) -> Result<()> {
    let triple = match target {
        Some(t) => t.to_owned(),
        None => std::env::var("TAURI_ENV_TARGET_TRIPLE").context(
            "no target: pass --target <triple> or run from `pnpm bundle` (TAURI_ENV_TARGET_TRIPLE)",
        )?,
    };
    let debug = std::env::var("TAURI_ENV_DEBUG").is_ok_and(|v| v == "true");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut build = Command::new(&cargo);
    build
        .current_dir(root)
        .args(["build", "-p", "rekindle-skin", "--bin", HELPER_NAME])
        .args(["--target", &triple]);
    if !debug {
        build.arg("--release");
    }
    let status = build.status().context("running cargo build")?;
    if !status.success() {
        return Err(anyhow!(
            "building {HELPER_NAME} for {triple} failed ({status})"
        ));
    }
    let profile = if debug { "debug" } else { "release" };
    let built = target_directory(root, &cargo)?
        .join(&triple)
        .join(profile)
        .join(lookup_name(&triple));
    let dir = root.join("src-tauri").join(BINARIES_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let staged = dir.join(staged_name(HELPER_NAME, &triple));
    std::fs::copy(&built, &staged)
        .with_context(|| format!("copying {} to {}", built.display(), staged.display()))?;
    println!("staged {}", staged.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        conf_problems, entry_problems, installed_name, staged_name, CONFIG_FLAG, HELPER_NAME,
    };
    use serde_json::json;

    #[test]
    fn staged_and_installed_names_follow_tauri() {
        let win = staged_name(HELPER_NAME, "x86_64-pc-windows-msvc");
        assert_eq!(win, "rekindle-skin-helper-x86_64-pc-windows-msvc.exe");
        assert_eq!(
            installed_name(&win, "x86_64-pc-windows-msvc"),
            "rekindle-skin-helper.exe"
        );
        let mac = staged_name(HELPER_NAME, "aarch64-apple-darwin");
        assert_eq!(mac, "rekindle-skin-helper-aarch64-apple-darwin");
        assert_eq!(
            installed_name(&mac, "aarch64-apple-darwin"),
            "rekindle-skin-helper"
        );
    }

    fn base() -> serde_json::Value {
        json!({"build": {"beforeBuildCommand": "pnpm build"}, "bundle": {}})
    }

    fn release() -> serde_json::Value {
        json!({
            "build": {"beforeBuildCommand": "pnpm build && cargo xtask stage-sidecar"},
            "bundle": {"externalBin": ["binaries/rekindle-skin-helper"]}
        })
    }

    #[test]
    fn the_configs_must_bundle_and_stage_the_helper() {
        assert!(conf_problems(&base(), &release()).is_empty());
        let mut none = release();
        none["bundle"] = json!({});
        assert_eq!(conf_problems(&base(), &none).len(), 1);
        let mut suffixed = release();
        suffixed["bundle"]["externalBin"] = json!(["binaries/rekindle-skin-helper.exe"]);
        // Missing the exact entry, and it installs as `….exe.exe` on Windows
        // and `….exe` elsewhere: every release triple is a hit.
        assert_eq!(conf_problems(&base(), &suffixed).len(), 5);
        let mut unstaged = release();
        unstaged["build"]["beforeBuildCommand"] = json!("pnpm build");
        assert_eq!(conf_problems(&base(), &unstaged).len(), 1);
        let mut no_frontend = release();
        no_frontend["build"]["beforeBuildCommand"] = json!("cargo xtask stage-sidecar");
        assert_eq!(conf_problems(&base(), &no_frontend).len(), 1);
        let mut in_base = base();
        in_base["bundle"]["externalBin"] = json!(["binaries/rekindle-skin-helper"]);
        assert_eq!(conf_problems(&in_base, &release()).len(), 1);
    }

    #[test]
    fn every_release_entry_point_passes_the_overlay() {
        let yml = format!(
            "      - name: Build\n        uses: tauri-apps/tauri-action@v0\n        with:\n          args: ${{{{ matrix.args }}}} {CONFIG_FLAG}\n"
        );
        let pkg = json!({"scripts": {"bundle": format!("tauri build {CONFIG_FLAG}")}});
        let good = "pnpm bundle\n";
        assert!(entry_problems(&yml, &pkg, good, good).is_empty());
        let bare_yml = yml.replace(CONFIG_FLAG, "");
        assert_eq!(entry_problems(&bare_yml, &pkg, good, good).len(), 1);
        let no_script = json!({"scripts": {}});
        assert_eq!(entry_problems(&yml, &no_script, good, good).len(), 1);
        let bare = "pnpm bundle\npnpm tauri build\n";
        assert_eq!(entry_problems(&yml, &pkg, bare, good).len(), 1);
        assert_eq!(
            entry_problems(&yml, &pkg, good, "pnpm tauri build").len(),
            1
        );
    }
}

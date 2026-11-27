//! `cargo xtask import-xfire-data --from <legacy/unpacked>` (plan E7.7.3
//! "Xfire app data from the same tree", xtask only): writes the app data
//! the owner's Xfire install tree supplies, each output with a `NOTICE`.
//!
//! 1. `icons.dll` → `src/assets/game-icons/xfire/<xfire_id>.png` ([`pe`],
//!    [`icon`]): the icon of an Xfire game is `XF_<SHORTNAME>.ICO`, where
//!    `<SHORTNAME>` is the first `ShortName` of the game's sections (in file
//!    order) that names an icon. Consumed by E7.2 "Game icons".
//! 2. `xfire_games.ini` + `default_games.json` →
//!    `crates/rekindle-game-detect/src/xfire_ids.json` ([`games`]), the
//!    `database id → Xfire game id` mapping. Consumed by E7.2.
//! 3. `sounds/{defaults,classic}.zip` → `crates/rekindle-skin/builtin/
//!    {defaults,classic}.rkskin` ([`sounds`]). Consumed by E7.7.4.
//!
//! The import is deterministic: the same tree writes the same bytes, so the
//! owner's replacement packs use the same command.

mod games;
mod icon;
mod pe;
mod sounds;
mod zip;

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::import_skin::BUILTIN_DIR;

/// Where the game icons go (plan E7.2: `src/assets/game-icons/<key>.png`,
/// key `xfire/<xfire_id>`).
const ICON_DIR: &str = "src/assets/game-icons/xfire";
const GAME_DETECT_SRC: &str = "crates/rekindle-game-detect/src";
const NOTICE: &str = "NOTICE";
/// The two Xfire sound packs and the built-in bank id each becomes.
const BANKS: [(&str, &str); 2] = [
    ("defaults", "sounds/defaults.zip"),
    ("classic", "sounds/classic.zip"),
];

fn read(from: &Path, rel: &str) -> Result<Vec<u8>> {
    std::fs::read(from.join(rel)).with_context(|| format!("reading {rel}"))
}

fn write(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(&path, bytes).with_context(|| format!("writing {rel}"))
}

/// The provenance every `NOTICE` opens with: what it covers, each input
/// with its BLAKE3, the owner's ruling and the licence boundary.
fn notice(covers: &str, inputs: &[(&str, &[u8])]) -> String {
    let mut s = format!("{covers}\n\nProvenance\n");
    s.push_str(
        "  Imported by `cargo xtask import-xfire-data` (plan E7.7.3) from the project owner's\n  \
         Xfire install tree, git commit ba066924 (\"xfire re'd\"; removed from the tree by\n  \
         5a20b508), restored into the git-ignored `legacy/unpacked/`:\n",
    );
    for (rel, bytes) in inputs {
        let _ = writeln!(
            s,
            "    legacy/unpacked/{rel}  BLAKE3 {}",
            blake3::hash(bytes).to_hex()
        );
    }
    s.push_str(
        "\nLicence\n  \
         This is Xfire, Inc. content; Xfire is defunct. The project owner ruled on 2026-10-15\n  \
         that Xfire's own assets may be used in Rekindle. Rekindle's MIT OR Apache-2.0 \
         licence does not cover\n  \
         it.\n",
    );
    s
}

fn import_icons(root: &Path, dll: &[u8], ini: &[u8], games: &games::Games) -> Result<()> {
    let icons = pe::icons(dll)?;
    let dir = root.join(ICON_DIR);
    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "png") {
                std::fs::remove_file(&path)?;
            }
        }
    }
    let mut used = std::collections::BTreeSet::new();
    let mut written = 0usize;
    for (id, game) in games {
        let Some(short) = game
            .short_names
            .iter()
            .map(|s| s.to_ascii_uppercase())
            .find(|s| icons.contains_key(s))
        else {
            continue;
        };
        let png = icon::to_png(&format!("XF_{short}.ICO"), icons[&short])?;
        write(root, &format!("{ICON_DIR}/{id}.png"), &png)?;
        used.insert(short);
        written += 1;
    }
    let unused: Vec<String> = icons
        .keys()
        .filter(|k| !used.contains(*k))
        .map(|k| format!("XF_{k}.ICO"))
        .collect();
    let mut n = notice(
        "Xfire game icons: `<xfire_id>.png`, one per Xfire game id (plan E7.2 \"Game icons\").",
        &[("icons.dll", dll), ("xfire_games.ini", ini)],
    );
    let _ = write!(
        n,
        "\nHow they were made\n  \
         `icons.dll` is read as PE resource data with `pelite`, never loaded. Its named resource\n  \
         type `ICONS` holds {} whole `.ICO` files named `XF_<SHORTNAME>.ICO`; an Xfire game's\n  \
         icon is the one named by the first `ShortName` of its `xfire_games.ini` sections, in file\n  \
         order. Each `.ICO`'s largest image is kept: a PNG copied, a DIB converted with its AND\n  \
         mask (a 32-bit DIB's own alpha when any is set).\n  \
         {written} files written; {} Xfire games have no icon; {} icons name no game and are not\n  \
         written:\n",
        icons.len(),
        games.len() - written,
        unused.len(),
    );
    for name in &unused {
        let _ = writeln!(n, "    {name}");
    }
    write(root, &format!("{ICON_DIR}/{NOTICE}"), n.as_bytes())?;
    println!("    wrote {written} icons and {ICON_DIR}/{NOTICE}");
    Ok(())
}

fn import_ids(root: &Path, ini: &[u8], games: &games::Games) -> Result<()> {
    let db_rel = format!("{GAME_DETECT_SRC}/default_games.json");
    let db = std::fs::read(root.join(&db_rel)).with_context(|| format!("reading {db_rel}"))?;
    let db_text = std::str::from_utf8(&db).context("default_games.json is not UTF-8")?;
    let mapped = games::map(db_text, games)?;
    write(
        root,
        &format!("{GAME_DETECT_SRC}/xfire_ids.json"),
        games::json(&mapped).as_bytes(),
    )?;
    let mut n = notice(
        "`xfire_ids.json`: Rekindle game database id -> Xfire game id (plan E7.2 \"Game icons\").",
        &[("xfire_games.ini", ini)],
    );
    let _ = writeln!(
        n,
        "\nHow it was made\n  \
         Each `default_games.json` game is matched by name and process: an Xfire game whose\n  \
         `LongName` equals the game's name (letters and digits, case folded) and whose\n  \
         `LauncherExe` or `DetectExe` is one of its process names. {} of the database's games\n  \
         match; the rest keep their slug icon.",
        mapped.len()
    );
    for m in &mapped {
        let _ = writeln!(n, "    {} {} -> {}", m.id, m.name, m.xfire_id);
    }
    n.push_str(
        "  Games with Xfire's `LauncherNetworkArgs` (the launcher's `steam://run` arguments when\n  \
         the game has a Steam app id and no connect template):\n",
    );
    for m in &mapped {
        if let Some(args) = &m.network_args {
            let _ = writeln!(n, "    {} {}: {args}", m.id, m.name);
        }
    }
    write(
        root,
        &format!("{GAME_DETECT_SRC}/xfire_ids.NOTICE"),
        n.as_bytes(),
    )?;
    println!(
        "    wrote {GAME_DETECT_SRC}/xfire_ids.json ({} games)",
        mapped.len()
    );
    Ok(())
}

fn import_bank(root: &Path, from: &Path, id: &str, rel: &str) -> Result<()> {
    let zip = read(from, rel)?;
    let bank = sounds::import(id, &zip)?;
    write(
        root,
        &format!("{BUILTIN_DIR}/{id}.rkskin"),
        &bank.canonical.bytes,
    )?;
    let mut n = notice(
        &format!(
            "Built-in sound bank `builtin:{id}` (`{id}.rkskin`, BLAKE3 {}): Xfire's \"{}\".",
            bank.canonical.address(),
            bank.long_name
        ),
        &[(rel, &zip)],
    );
    n.push_str(
        "\nHow it was made\n  \
         Each cue is the file `sounds.xml` names for its Xfire sound type, re-encoded to PCM16\n  \
         WAV by the skin validator. Xfire had no mention or busy sound: mention plays \"Receive\n  \
         Message\" (Xfire's sound for every incoming message) and busy plays \"Call End\". The\n  \
         package's one variant has an empty token file (the registry defaults).\n",
    );
    for (cue, file) in &bank.cues {
        let _ = writeln!(n, "    {} <- {file}", cue.id());
    }
    n.push_str("  Xfire sound types with no Rekindle cue (not imported):\n");
    for (ty, file) in &bank.dropped {
        let _ = writeln!(
            n,
            "    {ty} ({})",
            if file.is_empty() { "no file" } else { file }
        );
    }
    write(root, &format!("{BUILTIN_DIR}/{id}.NOTICE"), n.as_bytes())?;
    println!(
        "    wrote {BUILTIN_DIR}/{id}.rkskin ({})",
        bank.canonical.address()
    );
    Ok(())
}

/// Imports the Xfire app data under `from` into the workspace at `root`.
///
/// # Errors
/// A missing or malformed input, or an output that cannot be written.
pub fn import_xfire_data(root: &Path, from: &Path) -> Result<()> {
    let dll = read(from, "icons.dll")?;
    let ini = read(from, "xfire_games.ini")?;
    let games = games::read(&ini).context("xfire_games.ini")?;
    import_icons(root, &dll, &ini, &games)?;
    import_ids(root, &ini, &games)?;
    for (id, rel) in BANKS {
        import_bank(root, from, id, rel)?;
    }
    Ok(())
}

//! `xfire_games.ini`: Xfire's game list, read with a bounded line reader,
//! and the explicit `database id → Xfire game id` mapping of plan E7.2
//! "Game icons" built from it.
//!
//! The file (build 46139, UTF-8, CRLF) holds `[Modified]`, `[Version]`,
//! the unknown game `[-1]`, and one section per detection variant of a
//! game: `[<id>]` or `[<id>_<n>]` (`[7197_1]`, `[7197_2]` are both 0 A.D.,
//! id 7197). A game's keys used here are `LongName`, `ShortName`,
//! `LauncherExe` and `DetectExe` / `DetectExe[n]`; `;` starts a comment.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;

/// The largest INI read (build 46139's is 1.9 MiB).
const MAX_INI_BYTES: usize = 16 * 1024 * 1024;
/// The longest line.
const MAX_LINE: usize = 4096;
/// The most sections.
const MAX_SECTIONS: usize = 65_536;

/// One Xfire game, merged over its detection variants.
#[derive(Debug, Default)]
pub struct XfireGame {
    /// `ShortName`s in file order, each once.
    pub short_names: Vec<String>,
    pub long_names: BTreeSet<String>,
    /// Lower-case base names of `LauncherExe` and `DetectExe*`.
    pub exes: BTreeSet<String>,
}

/// Every Xfire game, by Xfire id.
pub type Games = BTreeMap<u32, XfireGame>;

/// The game id of a section name, `None` for a non-game section.
fn section_id(name: &str) -> Result<Option<u32>> {
    if matches!(name, "Modified" | "Version" | "-1") {
        return Ok(None);
    }
    let id = name.split_once('_').map_or(name, |(id, n)| {
        if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) {
            id
        } else {
            name
        }
    });
    let ok = !id.is_empty() && id.len() <= 9 && id.bytes().all(|c| c.is_ascii_digit());
    ensure!(ok, "section [{name}] is not [<id>] or [<id>_<n>]");
    Ok(Some(id.parse()?))
}

fn exe_base(value: &str) -> String {
    value
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Every game in the INI, by Xfire id.
///
/// # Errors
/// A file past the limits, not UTF-8, a malformed line or section.
pub fn read(bytes: &[u8]) -> Result<Games> {
    ensure!(bytes.len() <= MAX_INI_BYTES, "{} bytes", bytes.len());
    let text = std::str::from_utf8(bytes).context("not UTF-8")?;
    let mut games = Games::new();
    let mut seen = BTreeSet::new();
    let mut current: Option<u32> = None;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw).trim();
        ensure!(line.len() <= MAX_LINE, "line {} too long", i + 1);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            ensure!(seen.insert(name.to_owned()), "section [{name}] twice");
            ensure!(
                seen.len() <= MAX_SECTIONS,
                "more than {MAX_SECTIONS} sections"
            );
            current = section_id(name)?;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("line {}: neither a section nor key=value", i + 1);
        };
        let Some(id) = current else { continue };
        let key = key.split_once('[').map_or(key, |(k, _)| k);
        let value = value.trim().trim_end_matches('\u{200b}');
        let game = games.entry(id).or_default();
        match key {
            "LongName" => {
                game.long_names.insert(value.to_owned());
            }
            "ShortName" if !game.short_names.iter().any(|s| s == value) => {
                game.short_names.push(value.to_owned());
            }
            "LauncherExe" | "DetectExe" if !value.is_empty() => {
                game.exes.insert(exe_base(value));
            }
            _ => {}
        }
    }
    Ok(games)
}

/// A name compared by its lower-case ASCII letters and digits alone.
fn fold(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// One database game mapped to its Xfire game.
#[derive(Debug, PartialEq, Eq)]
pub struct Mapped {
    pub id: u32,
    pub name: String,
    pub xfire_id: u32,
}

/// The database's games (`default_games.json`) matched against `games`
/// **by name and process**: an Xfire game whose `LongName` folds equal to
/// the entry's name and whose launcher or detect executable is one of the
/// entry's process names. A game with no such Xfire game is left out (its
/// icon key stays its slug); one with two is refused.
///
/// # Errors
/// A malformed database or an ambiguous match.
pub fn map(database: &str, games: &Games) -> Result<Vec<Mapped>> {
    let db: Value = serde_json::from_str(database).context("default_games.json")?;
    let entries = db["games"].as_array().context("no games array")?;
    let mut out = Vec::new();
    for e in entries {
        let id = e["id"].as_u64().context("game without id")?;
        let name = e["name"].as_str().context("game without name")?;
        let procs: BTreeSet<String> = e["process_names"]
            .as_array()
            .context("game without process_names")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_ascii_lowercase)
            .collect();
        let hits: Vec<u32> = games
            .iter()
            .filter(|(_, g)| g.long_names.iter().any(|l| fold(l) == fold(name)))
            .filter(|(_, g)| !g.exes.is_disjoint(&procs))
            .map(|(x, _)| *x)
            .collect();
        match hits.as_slice() {
            [] => {}
            [xfire_id] => out.push(Mapped {
                id: u32::try_from(id)?,
                name: name.to_owned(),
                xfire_id: *xfire_id,
            }),
            many => bail!("{name} ({id}) matches Xfire games {many:?}"),
        }
    }
    out.sort_by_key(|m| m.id);
    Ok(out)
}

/// `xfire_ids.json`: `{"games": [{"id", "xfire_id"}, …]}` by database id.
#[must_use]
pub fn json(mapped: &[Mapped]) -> String {
    let games: Vec<Value> = mapped
        .iter()
        .map(|m| serde_json::json!({ "id": m.id, "xfire_id": m.xfire_id }))
        .collect();
    let mut s =
        serde_json::to_string_pretty(&serde_json::json!({ "games": games })).unwrap_or_default();
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const INI: &str = "[Modified]\r\nLast=1\r\n\r\n[Version]\r\nVersion=46139\r\n\r\n\
        [-1]\r\nLongName=Unknown Game\r\nShortName=unknown\r\n\r\n\
        [7197_1]\r\nLongName=0 A.D.\r\nShortName=0ad\r\nLauncherExe=pyrogenesis.exe\r\n\r\n\
        [7197_2]\r\nLongName=0 A.D.\r\nShortName=0ad\r\nDetectExe[0]=bin\\\\Pyro2.exe\r\n\
        ;LauncherExe=commented.exe\r\n\r\n\
        [5134]\r\nLongName=Team Fortress 2\r\nShortName=tf2\r\nLauncherExe=steam.exe\r\n\
        DetectExe=tf_win64.exe\r\n";

    #[test]
    fn variants_merge_under_one_id() {
        let games = read(INI.as_bytes()).unwrap();
        assert_eq!(games.keys().copied().collect::<Vec<_>>(), [5134, 7197]);
        let ad = &games[&7197];
        assert_eq!(ad.short_names, ["0ad"]);
        let exes: Vec<&str> = ad.exes.iter().map(String::as_str).collect();
        assert_eq!(exes, ["pyro2.exe", "pyrogenesis.exe"]);
        assert!(read(b"[abc]\r\nLongName=x\r\n").is_err());
        assert!(read(b"[1]\r\n[1]\r\n").is_err());
        assert!(read(b"[1]\r\nnot a pair\r\n").is_err());
    }

    #[test]
    fn a_game_maps_only_by_name_and_process() {
        let games = read(INI.as_bytes()).unwrap();
        let db = r#"{"games": [
            {"id": 9, "name": "0 A.D.", "process_names": ["Pyrogenesis.exe"]},
            {"id": 4100, "name": "Team Fortress 2", "process_names": ["hl2.exe"]},
            {"id": 7, "name": "Zero AD", "process_names": ["pyrogenesis.exe"]}
        ]}"#;
        let mapped = map(db, &games).unwrap();
        assert_eq!(
            mapped,
            [Mapped {
                id: 9,
                name: "0 A.D.".to_owned(),
                xfire_id: 7197
            }],
            "name alone (TF2) or process alone (Zero AD) does not map"
        );
        assert!(json(&mapped).contains("\"xfire_id\": 7197"));
    }
}

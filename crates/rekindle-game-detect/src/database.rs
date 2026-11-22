use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A game entry in the detection database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    pub id: u32,
    pub name: String,
    pub process_names: Vec<String>,
    pub icon: Option<String>,
    /// Steam application ID (for `steam://connect/` launch URIs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_app_id: Option<u32>,
    /// URI template for connecting to a server, e.g. `"steam://connect/{addr}"`.
    /// `{addr}` is replaced with the `ip:port` at launch time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_template: Option<String>,
    /// The OS's own copy of the game, started from a fixed path (plan
    /// E7.15 CH1: the native Chess of each OS is the test game).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeApps>,
    /// The query protocol this game's servers answer (plan E7.2 G4): a
    /// Servers view row of a game without one answers "This game has no
    /// server query".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_query: Option<crate::server_query::ServerQueryProtocol>,
    /// Xfire's id for this game, from `xfire_ids.json` (plan E7.2 "Game
    /// icons"); not part of the database JSON.
    #[serde(skip)]
    pub xfire_id: Option<u32>,
}

/// Where each OS keeps its own copy of a game. Every path is fixed in the
/// bundled database and absolute; nothing from outside joins it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NativeApps {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<MacApp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux: Option<LinuxApp>,
}

/// A macOS app bundle: launch services open `path`, and the bundle there
/// is ours only when its `Info.plist` names `bundle_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacApp {
    pub bundle_id: String,
    pub path: String,
}

/// A Linux executable, run by its full path with no arguments and no shell
/// (`/usr/games` is not always on `PATH`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxApp {
    pub path: String,
}

impl GameEntry {
    /// The key of this game's icon, `src/assets/game-icons/<key>.png`:
    /// `xfire/<xfire id>` when the game is in Xfire's list, else
    /// `slug/<icon>` (plan E7.2). Both ends hold the same database, so
    /// the key never crosses the wire; a frontend shows the generic glyph
    /// for a key with no file.
    #[must_use]
    pub fn icon_key(&self) -> Option<String> {
        self.xfire_id.map_or_else(
            || self.icon.as_ref().map(|slug| format!("slug/{slug}")),
            |xfire_id| Some(format!("xfire/{xfire_id}")),
        )
    }
}

/// Maps process names to game information.
///
/// Loaded from a JSON database file. The original Xfire used a 2MB INI file
/// (`xfire_games.ini`) with entries for hundreds of games.
pub struct GameDatabase {
    /// Lowercase process name -> `GameEntry`
    by_process: HashMap<String, GameEntry>,
}

impl GameDatabase {
    /// Load a game database from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let entries: GameDatabaseFile = serde_json::from_str(json)?;
        let mut by_process = HashMap::new();

        for entry in entries.games {
            for proc_name in &entry.process_names {
                by_process.insert(proc_name.to_lowercase(), entry.clone());
            }
        }

        Ok(Self { by_process })
    }

    /// Load the bundled default game database.
    ///
    /// Contains 50+ popular games. Users can extend this with custom JSON.
    pub fn bundled() -> Self {
        const DEFAULT_JSON: &str = include_str!("default_games.json");
        const XFIRE_IDS_JSON: &str = include_str!("xfire_ids.json");
        let mut db = Self::from_json(DEFAULT_JSON).expect("bundled game database is invalid JSON");
        let ids: XfireIdsFile =
            serde_json::from_str(XFIRE_IDS_JSON).expect("bundled xfire_ids.json is invalid JSON");
        db.apply_xfire_ids(&ids.games);
        db
    }

    /// The bundled database, loaded once for the process.
    pub fn shared() -> &'static Self {
        static SHARED: std::sync::OnceLock<GameDatabase> = std::sync::OnceLock::new();
        SHARED.get_or_init(Self::bundled)
    }

    /// The icon key of game `game_id` in the bundled database
    /// ([`GameEntry::icon_key`]); `None` for an unknown game.
    #[must_use]
    pub fn icon_key_for(game_id: u32) -> Option<String> {
        Self::shared()
            .lookup_by_id(game_id)
            .and_then(GameEntry::icon_key)
    }

    fn apply_xfire_ids(&mut self, ids: &[XfireId]) {
        let by_id: HashMap<u32, u32> = ids.iter().map(|m| (m.id, m.xfire_id)).collect();
        for entry in self.by_process.values_mut() {
            entry.xfire_id = by_id.get(&entry.id).copied();
        }
    }

    /// Create an empty database.
    pub fn empty() -> Self {
        Self {
            by_process: HashMap::new(),
        }
    }

    /// The ids of every game with Steam application id `app_id`, ascending
    /// (two entries may share one: a game and its successor under the same
    /// Steam app).
    #[must_use]
    pub fn ids_with_steam_app_id(&self, app_id: u32) -> std::collections::BTreeSet<u32> {
        self.by_process
            .values()
            .filter(|entry| entry.steam_app_id == Some(app_id))
            .map(|entry| entry.id)
            .collect()
    }

    /// Every entry, once per process name (an entry with two names comes
    /// twice; callers collect ids).
    pub fn entries(&self) -> impl Iterator<Item = &GameEntry> {
        self.by_process.values()
    }

    /// Look up a game by process name (case-insensitive).
    pub fn lookup_by_process(&self, process_name: &str) -> Option<&GameEntry> {
        self.by_process.get(&process_name.to_lowercase())
    }

    /// Look up a game by its unique numeric ID.
    pub fn lookup_by_id(&self, game_id: u32) -> Option<&GameEntry> {
        self.by_process.values().find(|e| e.id == game_id)
    }

    /// Get the number of games in the database.
    pub fn game_count(&self) -> usize {
        let unique: std::collections::HashSet<u32> =
            self.by_process.values().map(|e| e.id).collect();
        unique.len()
    }
}

#[derive(Deserialize)]
struct GameDatabaseFile {
    games: Vec<GameEntry>,
}

/// `xfire_ids.json`, written by `cargo xtask import-xfire-data` (plan E7.7.3).
#[derive(Deserialize)]
struct XfireIdsFile {
    games: Vec<XfireId>,
}

#[derive(Deserialize)]
struct XfireId {
    id: u32,
    xfire_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_and_lookup() {
        let json = r#"{
            "games": [
                {
                    "id": 4181,
                    "name": "Counter-Strike 2",
                    "process_names": ["cs2.exe", "cs2"],
                    "icon": "cs2"
                }
            ]
        }"#;

        let db = GameDatabase::from_json(json).unwrap();
        assert_eq!(db.game_count(), 1);

        let entry = db.lookup_by_process("cs2.exe").unwrap();
        assert_eq!(entry.name, "Counter-Strike 2");

        // Case insensitive
        let entry = db.lookup_by_process("CS2.EXE").unwrap();
        assert_eq!(entry.id, 4181);
    }

    /// Counter-Strike 2 is in Xfire's list (`xfire_ids.json`), so its
    /// icon is Xfire's; Valorant is not, so it keeps its slug; an unknown
    /// game has none.
    #[test]
    fn icon_keys_prefer_the_xfire_id_then_the_slug() {
        assert_eq!(
            GameDatabase::icon_key_for(4181).as_deref(),
            Some("xfire/8507")
        );
        let unmapped = GameDatabase::shared()
            .by_process
            .values()
            .find(|e| e.xfire_id.is_none())
            .expect("a game Xfire never listed");
        assert_eq!(
            unmapped.icon_key(),
            Some(format!("slug/{}", unmapped.icon.as_ref().unwrap()))
        );
        assert_eq!(GameDatabase::icon_key_for(1), None);
    }

    /// A launcher substitutes the address once: a template with no
    /// `{addr}`, or two, would launch somewhere the user did not pick.
    #[test]
    fn every_connect_template_holds_one_address() {
        let db = GameDatabase::bundled();
        for entry in db.by_process.values() {
            if let Some(template) = &entry.connect_template {
                assert_eq!(
                    template.matches("{addr}").count(),
                    1,
                    "{}: {template}",
                    entry.name
                );
            }
        }
    }
}

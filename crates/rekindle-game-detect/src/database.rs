use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A game entry in the detection database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEntry {
    pub id: u32,
    pub name: String,
    pub process_names: Vec<String>,
    pub icon: Option<String>,
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
        Self::from_json(DEFAULT_JSON).expect("bundled game database is invalid JSON")
    }

    /// Create an empty database.
    pub fn empty() -> Self {
        Self {
            by_process: HashMap::new(),
        }
    }

    /// Look up a game by process name (case-insensitive).
    pub fn lookup_by_process(&self, process_name: &str) -> Option<&GameEntry> {
        self.by_process.get(&process_name.to_lowercase())
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
}

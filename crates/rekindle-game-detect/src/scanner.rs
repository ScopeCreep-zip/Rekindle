use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::database::{GameDatabase, GameEntry};
use crate::platform;
use crate::rich_presence::{self, RichPresence};

/// A detected running game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedGame {
    pub game_id: u32,
    pub game_name: String,
    pub process_name: String,
    pub started_at_epoch_ms: u64,
    pub rich_presence: Option<RichPresence>,
}

/// Periodically scans running processes to detect known games.
pub struct GameDetector {
    database: GameDatabase,
    scan_interval: Duration,
    current_game: Option<DetectedGame>,
    system: sysinfo::System,
}

impl GameDetector {
    pub fn new(database: GameDatabase, scan_interval: Duration) -> Self {
        Self {
            database,
            scan_interval,
            current_game: None,
            system: sysinfo::System::new(),
        }
    }

    /// Resolve one process to a game entry.
    ///
    /// Tries the raw process name first, then the platform-derived
    /// candidates from [`platform::alternate_lookup_names`]: the Windows
    /// binary behind a Proton wrapper, a macOS `.app` bundle name, or
    /// the other `.exe` spelling.
    ///
    /// Split out of `scan_once` so it can be tested against synthetic
    /// processes — `scan_once` itself needs a live process table.
    fn resolve_entry(
        &self,
        proc_name: &str,
        exe_path: Option<&str>,
        cmd_args: &[String],
    ) -> Option<GameEntry> {
        self.database
            .lookup_by_process(proc_name)
            .or_else(|| {
                platform::alternate_lookup_names(proc_name, exe_path, cmd_args)
                    .iter()
                    .find_map(|alt| self.database.lookup_by_process(alt))
            })
            .cloned()
    }

    /// Perform a single scan of running processes.
    pub fn scan_once(&mut self) -> Option<DetectedGame> {
        // `remove_dead_processes: true`. This was `false`, and because
        // `self.system` outlives the scan, exited processes were never
        // dropped from the table — so once a game was detected it stayed
        // "running" for the rest of the session.
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::All, true);

        for process in self.system.processes().values() {
            let proc_name = process.name().to_string_lossy().to_string();
            let cmd_args: Vec<String> = process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect();
            let exe_path = process.exe().map(|p| p.to_string_lossy().to_string());

            let Some(entry) = self.resolve_entry(&proc_name, exe_path.as_deref(), &cmd_args) else {
                continue;
            };

            // Extract rich presence from process command-line args
            let rp = rich_presence::parse_connect_args(&cmd_args)
                .map(|(ip, port)| RichPresence::with_server(entry.id, ip, port));

            let game = DetectedGame {
                game_id: entry.id,
                game_name: entry.name.clone(),
                process_name: proc_name,
                started_at_epoch_ms: rekindle_utils::timestamp_ms(),
                rich_presence: rp,
            };
            self.current_game = Some(game.clone());
            return Some(game);
        }

        self.current_game = None;
        None
    }

    /// Start a background scanning loop. Returns a watch receiver for game state changes.
    pub fn start_scanning(
        mut self,
    ) -> (
        tokio::task::JoinHandle<()>,
        watch::Receiver<Option<DetectedGame>>,
    ) {
        let (tx, rx) = watch::channel(None);
        let interval = self.scan_interval;

        let handle = tokio::spawn(async move {
            loop {
                let detected = self.scan_once();
                let _ = tx.send(detected);
                tokio::time::sleep(interval).await;
            }
        });

        (handle, rx)
    }

    pub fn current_game(&self) -> Option<&DetectedGame> {
        self.current_game.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the shape of the bundled database's `.exe`-only entries —
    /// 11 of the 50 real ones look like this (Elden Ring, Skyrim, Path
    /// of Exile, …), which is exactly the set that name matching alone
    /// cannot find under Proton.
    fn exe_only_db() -> GameDatabase {
        GameDatabase::from_json(
            r#"{"games":[{"id":1,"name":"Elden Ring",
                 "process_names":["eldenring.exe"],"icon":null}]}"#,
        )
        .expect("valid fixture")
    }

    fn detector(db: GameDatabase) -> GameDetector {
        GameDetector::new(db, Duration::from_secs(30))
    }

    #[test]
    fn exact_process_name_still_matches() {
        let d = detector(exe_only_db());
        let hit = d.resolve_entry("eldenring.exe", None, &[]).expect("match");
        assert_eq!(hit.id, 1);
    }

    #[test]
    fn unrelated_process_does_not_match() {
        let d = detector(exe_only_db());
        assert!(d.resolve_entry("bash", Some("/bin/bash"), &[]).is_none());
    }

    /// The regression this wiring exists for: under Proton the process
    /// is `wine64` and the game is named only in the command line.
    /// Before `alternate_lookup_names` was wired in, this returned None.
    #[cfg(target_os = "linux")]
    #[test]
    fn proton_wrapped_game_is_found() {
        let d = detector(exe_only_db());
        let args = vec!["wine64".to_string(), "Z:\\games\\eldenring.exe".to_string()];
        let hit = d.resolve_entry("wine64", None, &args).expect("match");
        assert_eq!(hit.name, "Elden Ring");
    }

    /// macOS equivalent: the process is the inner binary, the database
    /// entry is the bundle name.
    #[cfg(target_os = "macos")]
    #[test]
    fn app_bundle_game_is_found() {
        let db = GameDatabase::from_json(
            r#"{"games":[{"id":2,"name":"Steam","process_names":["Steam"],"icon":null}]}"#,
        )
        .expect("valid fixture");
        let d = detector(db);
        assert!(d.resolve_entry("steam_osx", None, &[]).is_none());
        let hit = d
            .resolve_entry(
                "steam_osx",
                Some("/Applications/Steam.app/Contents/MacOS/steam_osx"),
                &[],
            )
            .expect("bundle name must resolve");
        assert_eq!(hit.id, 2);
    }

    /// Windows: the database may hold either spelling.
    #[cfg(target_os = "windows")]
    #[test]
    fn bare_name_finds_exe_entry() {
        let d = detector(exe_only_db());
        let hit = d.resolve_entry("eldenring", None, &[]).expect("match");
        assert_eq!(hit.id, 1);
    }
}

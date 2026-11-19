use std::sync::Arc;
use std::time::Duration;

use rekindle_utils::Clock;
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
    /// When the scan found it, on the detector's clock port.
    pub started_at_epoch_ms: u64,
    pub rich_presence: Option<RichPresence>,
}

/// Periodically scans running processes to detect known games.
pub struct GameDetector {
    database: GameDatabase,
    scan_interval: Duration,
    current_game: Option<DetectedGame>,
    system: sysinfo::System,
    /// The Host's clock port (plan C9.2): it stamps a game's start.
    clock: Arc<dyn Clock>,
}

impl GameDetector {
    pub fn new(database: GameDatabase, scan_interval: Duration, clock: Arc<dyn Clock>) -> Self {
        Self {
            database,
            scan_interval,
            current_game: None,
            system: crate::platform::process_table(),
            clock,
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
    ///
    /// The table is refreshed with names and executable paths only; a
    /// command line is read for one process at a time, and only for a
    /// candidate: a process that already matched by name or path (its
    /// `+connect` gives the rich presence), or one whose name says the
    /// game is named only in its arguments (a Wine/Proton loader,
    /// [`platform::game_named_in_args`]). sysinfo 0.32.1's
    /// `refresh_processes` loads no argv, so reading `.cmd()` after it
    /// always gave nothing (plan E7.2, 2026-10-24).
    pub fn scan_once(&mut self) -> Option<DetectedGame> {
        // `remove_dead_processes: true`: `self.system` outlives the scan,
        // so without it an exited game stayed "running" all session.
        self.system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
        );
        let table: Vec<(sysinfo::Pid, String, Option<String>)> = self
            .system
            .processes()
            .iter()
            .map(|(pid, p)| {
                let exe = p.exe().map(|e| e.to_string_lossy().to_string());
                (*pid, p.name().to_string_lossy().to_string(), exe)
            })
            .collect();

        for (pid, proc_name, exe_path) in table {
            let by_name = self.resolve_entry(&proc_name, exe_path.as_deref(), &[]);
            if by_name.is_none() && !platform::game_named_in_args(&proc_name) {
                continue;
            }
            let cmd_args = self.command_line(pid);
            let Some(entry) =
                by_name.or_else(|| self.resolve_entry(&proc_name, exe_path.as_deref(), &cmd_args))
            else {
                continue;
            };
            let game = DetectedGame {
                game_id: entry.id,
                game_name: entry.name.clone(),
                process_name: proc_name,
                started_at_epoch_ms: self.clock.now_ms(),
                rich_presence: presence_from_args(entry.id, &cmd_args),
            };
            self.current_game = Some(game.clone());
            return Some(game);
        }

        self.current_game = None;
        None
    }

    /// One candidate's command line: refreshed for that PID alone.
    fn command_line(&mut self, pid: sysinfo::Pid) -> Vec<String> {
        self.system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::new().with_cmd(sysinfo::UpdateKind::OnlyIfNotSet),
        );
        self.system
            .process(pid)
            .map(|p| {
                p.cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Scan in a task of `scope` until it shuts down, publishing every
    /// scan on the returned watch channel.
    pub fn start_scanning(
        mut self,
        scope: &std::sync::Arc<rekindle_lifecycle::SessionScope>,
    ) -> watch::Receiver<Option<DetectedGame>> {
        let (tx, rx) = watch::channel(None);
        let interval = self.scan_interval;
        scope.spawn_with_token_or_drop("game scan", move |stop| async move {
            loop {
                let detected = self.scan_once();
                // No receivers left means nobody is watching; keep the
                // scan cadence regardless, the scope decides when to stop.
                tx.send_replace(detected);
                if stop
                    .run_until_cancelled(tokio::time::sleep(interval))
                    .await
                    .is_none()
                {
                    return;
                }
            }
        });
        rx
    }

    pub fn current_game(&self) -> Option<&DetectedGame> {
        self.current_game.as_ref()
    }
}

/// The rich presence a matched game's command line gives: the server of
/// its `+connect`-style argument, when that forms a valid address.
fn presence_from_args(game_id: u32, cmd_args: &[String]) -> Option<RichPresence> {
    rich_presence::parse_connect_args(cmd_args)
        .map(|(ip, port)| RichPresence::with_server(game_id, ip, port))
        .filter(|rp| rp.server_address().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Source game's `+connect` becomes a joinable `ServerAddress`
    /// (plan E7.2 argv fix).
    #[test]
    fn a_connect_argument_yields_a_server_address() {
        let args: Vec<String> = [
            "hl2.exe",
            "-game",
            "cstrike",
            "+connect",
            "203.0.113.7:27015",
        ]
        .map(String::from)
        .to_vec();
        let rp = presence_from_args(4181, &args).expect("rich presence");
        let server = rp.server_address().expect("a valid address");
        assert_eq!(server.to_string(), "203.0.113.7:27015");
        assert!(presence_from_args(4181, &["hl2.exe".to_string()]).is_none());
    }

    /// The bug: the names-only table holds no argv; a candidate's command
    /// line is loaded by its own refresh (this test process stands in).
    #[test]
    fn a_candidates_command_line_is_read_after_a_names_only_refresh() {
        let mut d = detector(exe_only_db());
        let me = sysinfo::get_current_pid().expect("own pid");
        d.system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::new(),
        );
        assert!(d.system.process(me).expect("listed").cmd().is_empty());
        let args = d.command_line(me);
        assert!(!args.is_empty(), "the candidate's argv is read");
    }

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
        GameDetector::new(
            db,
            Duration::from_secs(30),
            Arc::new(rekindle_utils::SystemClock),
        )
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

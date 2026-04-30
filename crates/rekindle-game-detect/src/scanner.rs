use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::database::GameDatabase;
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

    /// Perform a single scan of running processes.
    pub fn scan_once(&mut self) -> Option<DetectedGame> {
        self.system
            .refresh_processes(sysinfo::ProcessesToUpdate::All, false);

        for process in self.system.processes().values() {
            let proc_name = process.name().to_string_lossy().to_string();
            if let Some(entry) = self.database.lookup_by_process(&proc_name) {
                // Extract rich presence from process command-line args
                let cmd_args: Vec<String> = process
                    .cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().to_string())
                    .collect();
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

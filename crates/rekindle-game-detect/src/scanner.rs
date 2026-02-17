use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::database::GameDatabase;

/// A detected running game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedGame {
    pub game_id: u32,
    pub game_name: String,
    pub process_name: String,
    pub started_at_epoch_ms: u64,
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
        let processes: Vec<String> = self
            .system
            .processes()
            .values()
            .map(|p| p.name().to_string_lossy().to_string())
            .collect();

        for proc_name in &processes {
            if let Some(entry) = self.database.lookup_by_process(proc_name) {
                let game = DetectedGame {
                    game_id: entry.id,
                    game_name: entry.name.clone(),
                    process_name: proc_name.clone(),
                    started_at_epoch_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis().try_into().unwrap_or(u64::MAX),
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

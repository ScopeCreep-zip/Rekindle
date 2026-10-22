//! Test fixture: a temp data root and the host's empty identity slots.

use std::sync::Arc;

use rekindle_db::paths::DataRoot;
use rekindle_db::DbHandle;
use rekindle_idempotency::attempts::AttemptLimiter;
use rekindle_vault::VaultHandle;

use super::identity::Stores;

pub struct Fixture {
    _dir: tempfile::TempDir,
    pub root: DataRoot,
    pub db: Arc<DbHandle>,
    pub vault: VaultHandle,
    pub attempts: AttemptLimiter<String>,
}

/// A data root laid out under `dir` (nothing created).
pub fn temp_root(dir: &tempfile::TempDir) -> DataRoot {
    let data = dir.path().join("data");
    DataRoot {
        config: dir.path().join("config"),
        logs: dir.path().join("logs"),
        state: data.join("state"),
        data,
    }
}

impl Fixture {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = temp_root(&dir);
        std::fs::create_dir_all(&root.state).unwrap();
        Self {
            _dir: dir,
            root,
            db: Arc::new(DbHandle::default()),
            vault: rekindle_vault::new_handle(),
            attempts: AttemptLimiter::default(),
        }
    }

    pub fn stores(&self) -> Stores<'_> {
        Stores {
            db: &self.db,
            vault: &self.vault,
            attempts: &self.attempts,
        }
    }
}

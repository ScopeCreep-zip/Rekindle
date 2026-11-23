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
    pub device_vault: VaultHandle,
    pub attempts: AttemptLimiter<String>,
}

// The temp root lives in the public test fixture (plan GI3).
pub use super::test_fixtures::temp_root;

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
            device_vault: rekindle_vault::new_handle(),
            attempts: AttemptLimiter::default(),
        }
    }

    pub fn stores(&self) -> Stores<'_> {
        Stores {
            db: &self.db,
            vault: &self.vault,
            device_vault: &self.device_vault,
            attempts: &self.attempts,
        }
    }
}

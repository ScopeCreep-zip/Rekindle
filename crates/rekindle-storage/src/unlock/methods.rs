//! Unlock method enrollment registry.
//!
//! Stored as `unlock_methods.json` OUTSIDE the vault (readable before
//! the vault is opened). Contains no secrets — only the list of enrolled
//! unlock methods so the daemon knows which paths to try.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{StorageError, StorageResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockMethodEntry {
    pub id: String,
    pub method_type: String,
    pub display_name: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UnlockMethodsRegistry {
    pub methods: Vec<UnlockMethodEntry>,
}

impl UnlockMethodsRegistry {
    pub fn load(state_dir: &Path) -> StorageResult<Self> {
        let path = state_dir.join("unlock_methods.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let json = std::fs::read_to_string(&path)
            .map_err(|e| StorageError::SessionMetaParse(format!("unlock_methods: {e}")))?;
        serde_json::from_str(&json)
            .map_err(|e| StorageError::SessionMetaParse(format!("unlock_methods: {e}")))
    }

    pub fn save(&self, state_dir: &Path) -> StorageResult<()> {
        let path = state_dir.join("unlock_methods.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| StorageError::SessionMetaParse(format!("serialize: {e}")))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json).map_err(|e| StorageError::VaultCreationFailed {
            reason: format!("write: {e}"),
        })?;
        std::fs::rename(&tmp, &path).map_err(|e| StorageError::VaultCreationFailed {
            reason: format!("rename: {e}"),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn add(&mut self, entry: UnlockMethodEntry) {
        self.methods.retain(|m| m.id != entry.id);
        self.methods.push(entry);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.methods.len();
        self.methods.retain(|m| m.id != id);
        self.methods.len() < before
    }

    pub fn has(&self, id: &str) -> bool {
        self.methods.iter().any(|m| m.id == id)
    }

    pub fn list(&self) -> &[UnlockMethodEntry] {
        &self.methods
    }
}

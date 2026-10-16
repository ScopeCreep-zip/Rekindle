//! The identity registry, `state/identities.json` (plan D1): which
//! identities this data root holds, readable before any unlock (the login
//! picker and `IdentityList` read it). It holds no secret: a public key, a
//! display name and when the identity was created.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// One identity this root holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityEntry {
    pub public_key: String,
    pub display_name: String,
    /// Unix milliseconds.
    pub created_at: i64,
}

/// The identities a root holds, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub identities: Vec<IdentityEntry>,
}

/// Reading or writing the registry failed.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("identity registry: {0}")]
    Io(#[from] std::io::Error),
    #[error("identity registry is not valid JSON: {0}")]
    Malformed(#[from] serde_json::Error),
}

impl Registry {
    /// The entry for `public_key`.
    #[must_use]
    pub fn get(&self, public_key: &str) -> Option<&IdentityEntry> {
        self.identities.iter().find(|e| e.public_key == public_key)
    }

    /// Add or replace the entry for `entry.public_key`.
    pub fn upsert(&mut self, entry: IdentityEntry) {
        match self
            .identities
            .iter_mut()
            .find(|e| e.public_key == entry.public_key)
        {
            Some(existing) => *existing = entry,
            None => self.identities.push(entry),
        }
    }

    /// Remove `public_key`'s entry; returns whether it was there.
    pub fn remove(&mut self, public_key: &str) -> bool {
        let before = self.identities.len();
        self.identities.retain(|e| e.public_key != public_key);
        self.identities.len() != before
    }
}

/// Load the registry at `path`; a missing file is an empty registry.
///
/// # Errors
/// I/O, or a file that is not a registry.
pub fn load(path: &Path) -> Result<Registry, RegistryError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e.into()),
    }
}

/// Write the registry atomically (a 0600 temp file, synced, renamed over).
///
/// # Errors
/// I/O or serialization.
pub fn save(path: &Path, registry: &Registry) -> Result<(), RegistryError> {
    use std::io::Write;
    let json = serde_json::to_vec_pretty(registry)?;
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&json)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pk: &str) -> IdentityEntry {
        IdentityEntry {
            public_key: pk.into(),
            display_name: format!("name-{pk}"),
            created_at: 1,
        }
    }

    #[test]
    fn a_registry_round_trips_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identities.json");
        assert_eq!(load(&path).unwrap(), Registry::default());
        let mut reg = Registry::default();
        reg.upsert(entry("a"));
        reg.upsert(entry("b"));
        reg.upsert(IdentityEntry {
            display_name: "renamed".into(),
            ..entry("a")
        });
        save(&path, &reg).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back, reg);
        assert_eq!(back.get("a").unwrap().display_name, "renamed");
        assert_eq!(back.identities.len(), 2);
        let mut back = back;
        assert!(back.remove("a"));
        assert!(!back.remove("a"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn a_malformed_registry_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identities.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(matches!(load(&path), Err(RegistryError::Malformed(_))));
    }
}

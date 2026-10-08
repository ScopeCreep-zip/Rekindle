//! Admin policy: the one loader, used at startup and by `PolicyReload`.
//!
//! Two layers, each of which can only tighten what the one before it set:
//! `policy.toml` in the system config dir (`/etc/rekindle`, or
//! `%ProgramData%\rekindle` on Windows; the administrator) and in the
//! user config dir (`rekindle_utils::config_layers::{system_dir, user_dir}`). A layer that exists but does not
//! parse is an error, never a silent default — a policy file that is
//! present is a constraint someone meant to impose (plan C1, WS12.6).

use std::path::{Path, PathBuf};

/// Active authorization policy loaded from disk.
///
/// Admin policy constraints that cannot be overridden by user config.
/// Fields are additive: they set minimums/maximums, they never disable
/// features that users enabled.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Minimum allowed hop_count for any safety profile.
    pub min_hop_count: Option<u8>,
    /// Maximum allowed gossip TTL.
    pub max_gossip_ttl: Option<u8>,
}

/// A policy layer that exists but could not be used.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy {path}: read failed: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("policy {path}: parse failed: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("policy: {0}")]
    Dir(#[from] rekindle_utils::config_layers::LayerError),
}

/// Load the system layer, then the user layer, merged so the result is at
/// least as strict as each. Absent layers contribute nothing.
///
/// # Errors
/// A config directory cannot be resolved, or a layer that exists cannot be
/// read or parsed.
pub fn load_layered() -> Result<PolicyConfig, PolicyError> {
    use rekindle_utils::config_layers::{system_dir, user_dir};
    let mut policy = PolicyConfig::default();
    for path in [
        system_dir()?.join("policy.toml"),
        user_dir()?.join("policy.toml"),
    ] {
        if let Some(layer) = load_layer(&path)? {
            merge(&mut policy, &layer);
            tracing::info!(path = %path.display(), "policy layer loaded");
        }
    }
    Ok(policy)
}

fn load_layer(path: &Path) -> Result<Option<PolicyConfig>, PolicyError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => toml::from_str(&contents)
            .map(Some)
            .map_err(|source| PolicyError::Parse {
                path: path.to_path_buf(),
                source,
            }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PolicyError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Merge a loaded layer into the active policy.
///
/// Constraint direction is one-way: each successive layer can only
/// tighten constraints, never relax them. A user who sets
/// `min_hop_count = 0` when the system requires `2` gets `2`; one who sets
/// `4` gets `4`.
fn merge(active: &mut PolicyConfig, loaded: &PolicyConfig) {
    // min_hop_count: higher value wins (more privacy, never less)
    match (active.min_hop_count, loaded.min_hop_count) {
        (Some(a), Some(b)) => active.min_hop_count = Some(a.max(b)),
        (None, Some(b)) => active.min_hop_count = Some(b),
        _ => {}
    }
    // max_gossip_ttl: lower value wins (more restrictive, never more)
    match (active.max_gossip_ttl, loaded.max_gossip_ttl) {
        (Some(a), Some(b)) => active.max_gossip_ttl = Some(a.min(b)),
        (None, Some(b)) => active.max_gossip_ttl = Some(b),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_tightens_constraints() {
        let mut active = PolicyConfig {
            min_hop_count: Some(1),
            max_gossip_ttl: Some(5),
        };
        let loaded = PolicyConfig {
            min_hop_count: Some(2),
            max_gossip_ttl: Some(3),
        };
        merge(&mut active, &loaded);
        assert_eq!(active.min_hop_count, Some(2)); // higher = more privacy
        assert_eq!(active.max_gossip_ttl, Some(3)); // lower = more restrictive
    }

    #[test]
    fn merge_does_not_loosen() {
        let mut active = PolicyConfig {
            min_hop_count: Some(3),
            max_gossip_ttl: Some(2),
        };
        let loaded = PolicyConfig {
            min_hop_count: Some(1),
            max_gossip_ttl: Some(8),
        };
        merge(&mut active, &loaded);
        assert_eq!(active.min_hop_count, Some(3)); // not lowered
        assert_eq!(active.max_gossip_ttl, Some(2)); // not raised
    }

    #[test]
    fn absent_layer_contributes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_layer(&dir.path().join("policy.toml")).unwrap(), None);
    }

    /// A present but malformed policy is an error, not a default.
    #[test]
    fn malformed_layer_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, "min_hop_count = \"two\"").unwrap();
        assert!(matches!(load_layer(&path), Err(PolicyError::Parse { .. })));
        std::fs::write(&path, "unknown_field = 1").unwrap();
        assert!(matches!(load_layer(&path), Err(PolicyError::Parse { .. })));
    }
}

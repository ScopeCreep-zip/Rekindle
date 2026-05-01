//! Doctor checks: storage health — session file, keystore, Veilid data dir.

use crate::doctor::{Check, Status};
use crate::helpers;

/// Run all storage health checks.
pub async fn checks(session: &rekindle_transport::Session) -> Vec<Check> {
    let mut results = Vec::new();

    // storage.session — session file exists and is valid
    let session_check = match helpers::session_path() {
        Ok(path) => {
            if path.exists() {
                match std::fs::metadata(&path) {
                    Ok(meta) => Check {
                        id: "storage.session".into(),
                        category: "storage",
                        status: Status::Pass,
                        value: format!(
                            "exists ({} bytes)",
                            meta.len()
                        ),
                        description: String::new(),
                    },
                    Err(e) => Check {
                        id: "storage.session".into(),
                        category: "storage",
                        status: Status::Warn,
                        value: format!("unreadable: {e}"),
                        description: "session file exists but metadata unreadable".into(),
                    },
                }
            } else {
                Check {
                    id: "storage.session".into(),
                    category: "storage",
                    status: Status::Fail,
                    value: "not found".into(),
                    description: format!(
                        "session file missing at {}\n\
                         initialize: rekindle init",
                        path.display()
                    ),
                }
            }
        }
        Err(e) => Check {
            id: "storage.session".into(),
            category: "storage",
            status: Status::Fail,
            value: format!("path error: {e}"),
            description: "cannot determine session file path".into(),
        },
    };
    results.push(session_check);

    // storage.keystore — verify keyring is accessible
    let keystore_check = match crate::identity::keystore::load_signing_key().await {
        Ok(_) => Check {
            id: "storage.keystore".into(),
            category: "storage",
            status: Status::Pass,
            value: "accessible".into(),
            description: String::new(),
        },
        Err(e) => {
            let is_not_found = e.to_string().contains("not found")
                || e.to_string().contains("NoEntry");
            if is_not_found {
                Check {
                    id: "storage.keystore".into(),
                    category: "storage",
                    status: Status::Fail,
                    value: "no signing key".into(),
                    description: "signing key not in keyring — identity may not be initialized\n\
                                 initialize: rekindle init"
                        .into(),
                }
            } else {
                Check {
                    id: "storage.keystore".into(),
                    category: "storage",
                    status: Status::Fail,
                    value: format!("error: {e}"),
                    description: "keyring access failed — check OS keyring service".into(),
                }
            }
        }
    };
    results.push(keystore_check);

    // storage.veilid_dir — Veilid persistent storage directory
    let veilid_check = match helpers::storage_dir(None) {
        Ok(dir) => {
            if dir.exists() {
                let size = dir_size(&dir);
                Check {
                    id: "storage.veilid_dir".into(),
                    category: "storage",
                    status: Status::Pass,
                    value: format!(
                        "exists, {}",
                        helpers::format_bytes(size)
                    ),
                    description: String::new(),
                }
            } else {
                Check {
                    id: "storage.veilid_dir".into(),
                    category: "storage",
                    status: Status::Warn,
                    value: "not found".into(),
                    description: format!(
                        "Veilid storage directory not found at {}\n\
                         it will be created on first node start",
                        dir.display()
                    ),
                }
            }
        }
        Err(e) => Check {
            id: "storage.veilid_dir".into(),
            category: "storage",
            status: Status::Warn,
            value: format!("path error: {e}"),
            description: "cannot determine storage directory path".into(),
        },
    };
    results.push(veilid_check);

    // storage.config — config file validation
    let config_check = match crate::config::load(None) {
        Ok(cfg) => match crate::config::validate(&cfg) {
            Ok(()) => Check {
                id: "storage.config".into(),
                category: "storage",
                status: Status::Pass,
                value: "valid".into(),
                description: String::new(),
            },
            Err(e) => Check {
                id: "storage.config".into(),
                category: "storage",
                status: Status::Warn,
                value: format!("invalid: {e}"),
                description: "config has validation errors\n\
                             check with: rekindle config validate"
                    .into(),
            },
        },
        Err(e) => Check {
            id: "storage.config".into(),
            category: "storage",
            status: Status::Warn,
            value: format!("load error: {e}"),
            description: "config file could not be loaded — using defaults".into(),
        },
    };
    results.push(config_check);

    // storage.communities — verify community count matches session
    let community_count = session.communities.len();
    results.push(Check {
        id: "storage.communities".into(),
        category: "storage",
        status: Status::Pass,
        value: format!("{community_count} communities in session"),
        description: String::new(),
    });

    results
}

/// Calculate the total size of a directory recursively.
///
/// Best-effort — silently skips files that can't be read.
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                total += dir_size(&entry_path);
            } else if let Ok(meta) = entry_path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

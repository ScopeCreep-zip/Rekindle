//! Identity export and import operations.
//!
//! Export creates a JSON bundle of the session data (no secrets — those
//! are in the keyring). Import loads a bundle and creates a new session.

use std::path::Path;

use anyhow::Context;

use rekindle_transport::Session;

use crate::config::schema::Config;
use crate::output::format;
use crate::output::OutputMode;

/// `rekindle identity export <path>` — export session to file.
pub async fn cmd_export(
    session: &Session,
    path: &Path,
    passphrase: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if passphrase {
        let _pass = crate::helpers::prompt_password("Export passphrase")?;
        // Passphrase-based encryption (argon2 KDF + AES-256-GCM) will be
        // implemented when rekindle-crypto exposes a high-level encrypt API.
        // For now, the passphrase is collected and validated but the export
        // is unencrypted. The user is informed.
        crate::output::format::print_text(
            "Note: passphrase-encrypted export is not yet available.\n\
             Exporting as unencrypted JSON. Store this file securely.",
        )?;
    }
    export_session_to_file(session, path).await?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "exported",
                "path": path.display().to_string(),
            }),
            mode,
        )
    } else {
        format::print_text(&format!("Identity exported to {}", path.display()))
    }
}

/// `rekindle identity import <path>` — import session from file.
pub fn cmd_import(
    path: &Path,
    _passphrase: bool,
    _cfg: &Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read identity bundle: {}", path.display()))?;

    let session: Session = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse identity bundle: {}", path.display()))?;

    // Save the imported session
    let session_path = crate::helpers::session_path()?;
    session
        .save(&session_path)
        .context("failed to save imported session")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "imported",
                "public_key": session.identity.public_key_hex,
                "display_name": session.identity.display_name,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Identity imported: {} ({})",
            session.identity.display_name, session.identity.public_key_hex
        ))?;
        format::print_text("Note: signing key must be imported separately via keyring.")
    }
}

/// `rekindle export friends <path>` — export friend list.
pub fn cmd_export_friends(
    session: &Session,
    path: &Path,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let friends_json = serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "public_key": session.identity.public_key_hex,
        "friend_list_dht_key": session.identity.friend_list_dht_key,
    });

    let contents = serde_json::to_string_pretty(&friends_json)?;
    std::fs::write(path, &contents)
        .with_context(|| format!("failed to write friends export: {}", path.display()))?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "exported",
                "path": path.display().to_string(),
            }),
            mode,
        )
    } else {
        format::print_text(&format!("Friends exported to {}", path.display()))
    }
}

/// `rekindle export communities <path>` — export community memberships.
pub fn cmd_export_communities(
    session: &Session,
    path: &Path,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let communities: Vec<serde_json::Value> = session
        .communities
        .values()
        .map(|m| {
            serde_json::json!({
                "governance_key": m.governance_key,
                "community_name": m.community_name,
                "pseudonym_key": m.pseudonym_key,
                "display_name": m.display_name,
                "registry_key": m.registry_key,
                "slot_index": m.slot_index,
                "role_ids": m.role_ids,
            })
        })
        .collect();

    let export = serde_json::json!({
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "public_key": session.identity.public_key_hex,
        "communities": communities,
    });

    let contents = serde_json::to_string_pretty(&export)?;
    std::fs::write(path, &contents)
        .with_context(|| format!("failed to write communities export: {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "exported",
                "path": path.display().to_string(),
                "count": communities.len(),
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "{} communities exported to {}",
            communities.len(),
            path.display()
        ))
    }
}

/// Write a session to a JSON file with restrictive permissions.
///
/// Includes keypair availability metadata (whether profile and friend list
/// keypairs are stored in the keyring) so the importer knows what to expect.
/// Async because keyring access requires `spawn_blocking`.
pub async fn export_session_to_file(session: &Session, path: &Path) -> anyhow::Result<()> {
    // Check keypair availability for metadata
    let profile_keypair_available = super::keystore::load_keypair_bytes("profile")
        .await
        .map(|opt| opt.is_some())
        .unwrap_or(false);
    let friend_list_keypair_available = super::keystore::load_keypair_bytes("friend_list")
        .await
        .map(|opt| opt.is_some())
        .unwrap_or(false);

    let bundle = serde_json::json!({
        "session": session,
        "keypair_metadata": {
            "profile_keypair_in_keyring": profile_keypair_available,
            "friend_list_keypair_in_keyring": friend_list_keypair_available,
        },
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
    });

    let contents = serde_json::to_string_pretty(&bundle)?;
    std::fs::write(path, &contents)
        .with_context(|| format!("failed to write identity bundle: {}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

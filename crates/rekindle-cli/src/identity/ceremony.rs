//! Identity creation ceremony (`rekindle init`) and destructive operations.
//!
//! The init ceremony is the first thing a new user runs. It orchestrates
//! the transport's `create_identity` operation, stores credentials in the
//! OS keyring, builds a `Session`, and persists it to disk.

use anyhow::Context;
use clap::Parser;
use tracing::info;
use zeroize::Zeroize;

use rekindle_transport::operations::identity;
use rekindle_transport::session::{Session, SessionIdentity};

use crate::cli::InitArgs;
use crate::config::schema::Config;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Execute `rekindle init` — the full identity creation ceremony.
pub async fn cmd_init(
    args: &InitArgs,
    cfg: &Config,
    existing_session: Option<&Session>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // Check if already initialized
    if existing_session.is_some() {
        anyhow::bail!(
            "identity already exists\n\
             to re-initialize, first destroy: rekindle identity destroy\n\
             or factory reset: rekindle init --wipe-all-data"
        );
    }

    // Resolve display name
    let display_name = if args.non_interactive {
        args.display_name
            .as_deref()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "--display-name is required in non-interactive mode"
                )
            })?
            .to_string()
    } else {
        helpers::resolve_display_name(args.display_name.as_deref())?
    };

    let display_name = helpers::validate_display_name(&display_name)?;

    // Step 1: Start transport node for the ceremony
    format::step_header(1, 5, "Starting transport node")?;

    let transport_config = cfg.to_transport_config(&crate::cli::Cli::parse_from(["rekindle"]))?;
    let handler = std::sync::Arc::new(NullHandler);
    let node = rekindle_transport::TransportNode::start(transport_config, handler)
        .await
        .context("failed to start transport node for identity ceremony")?;

    format::step_done("transport node started")?;

    // Step 2: Execute identity creation ceremony
    format::step_header(2, 5, "Creating identity")?;

    let mut created = identity::create_identity(&node, &display_name, "Hello from Rekindle!")
        .await
        .context("identity creation failed")?;

    info!(
        public_key = %created.public_key_hex,
        profile = %created.profile_dht_key,
        "identity created"
    );

    format::step_done(&format!(
        "identity: {}",
        helpers::abbreviate_key(&created.public_key_hex)
    ))?;

    // Step 3: Store credentials in keyring
    format::step_header(3, 5, "Securing credentials")?;

    super::keystore::store_signing_key(&created.signing_key_bytes).await?;
    super::keystore::store_keypair_bytes("profile", &created.profile_keypair_bytes).await?;
    super::keystore::store_keypair_bytes("friend_list", &created.friend_list_keypair_bytes)
        .await?;

    // Zeroize the signing key immediately after keyring storage
    created.signing_key_bytes.zeroize();

    format::step_done("credentials stored in OS keyring")?;

    // Step 4: Build and persist session
    format::step_header(4, 5, "Saving session")?;

    let session = Session::new(SessionIdentity {
        public_key_hex: created.public_key_hex.clone(),
        display_name: display_name.clone(),
        profile_dht_key: created.profile_dht_key.clone(),
        mailbox_dht_key: created.mailbox_dht_key.clone(),
        friend_list_dht_key: created.friend_list_dht_key.clone(),
        profile_keypair_bytes: None, // stored in keyring, not in session JSON
        friend_list_keypair_bytes: None,
    });

    let session_path = helpers::session_path()?;
    session
        .save(&session_path)
        .context("failed to save session")?;

    format::step_done(&format!("session saved to {}", session_path.display()))?;

    // Step 5: Optional export
    format::step_header(5, 5, "Exporting identity bundle")?;
    if let Some(ref export_path) = args.export_identity {
        super::export::export_session_to_file(&session, export_path).await?;
        format::step_done(&format!("exported to {}", export_path.display()))?;
    } else {
        format::step_skip("no --export-identity path provided")?;
    }

    // Shutdown node
    node.shutdown()
        .await
        .context("failed to shutdown transport node")?;

    // Summary
    if mode.is_structured() {
        crate::output::format::print_structured(
            &serde_json::json!({
                "status": "created",
                "public_key": created.public_key_hex,
                "display_name": display_name,
                "profile_dht_key": created.profile_dht_key,
                "mailbox_dht_key": created.mailbox_dht_key,
                "friend_list_dht_key": created.friend_list_dht_key,
            }),
            mode,
        )?;
    } else {
        format::print_text("\nIdentity created successfully.")?;
        format::print_text(&format!("  Public key: {}", created.public_key_hex))?;
        format::print_text(&format!("  Display name: {display_name}"))?;
        format::print_text("\nNext steps:")?;
        format::print_text("  rekindle status              — check node health")?;
        format::print_text("  rekindle community create    — create a community")?;
        format::print_text("  rekindle community join      — join with an invite")?;
        format::print_text("  rekindle friend add          — add a friend")?;
    }

    Ok(())
}

/// `rekindle identity rotate` — rotate the Ed25519 identity keypair.
pub async fn cmd_rotate(
    handle: &TransportHandle,
    session: &Session,
    force: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if !force {
        let confirmed = helpers::confirm(
            "Rotate identity keypair? All peers will need to re-verify your identity.",
        )?;
        if !confirmed {
            format::print_text("Cancelled.")?;
            return Ok(());
        }
    }

    let signing_key_bytes = super::keystore::load_signing_key().await?;

    let result = identity::rotate_identity(handle.node(), session, &signing_key_bytes)
        .await
        .context("identity rotation failed")?;

    // Store new key in keyring
    super::keystore::store_signing_key(&result.new_signing_key_bytes).await?;

    // Update session with new public key
    let session_path = helpers::session_path()?;
    let mut updated_session = session.clone();
    updated_session.identity.public_key_hex.clone_from(&result.new_public_key_hex);
    updated_session.save(&session_path)?;

    if mode.is_structured() {
        crate::output::format::print_structured(
            &serde_json::json!({
                "status": "rotated",
                "new_public_key": result.new_public_key_hex,
                "friends_notified": result.friends_notified,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Identity rotated. New public key: {}",
            result.new_public_key_hex
        ))?;
        format::print_text(&format!(
            "  {} friends notified",
            result.friends_notified
        ))
    }
}

/// `rekindle identity destroy` — irreversibly destroy the local identity.
pub async fn cmd_destroy(
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let confirmed = helpers::confirm_destructive(
        "This will permanently destroy your identity. All communities, friends, \
         and message history will be lost. This cannot be undone.",
        "destroy my identity",
    )?;

    if !confirmed {
        format::print_text("Cancelled.")?;
        return Ok(());
    }

    // Close all DHT records
    identity::destroy_identity(handle.node(), session)
        .await
        .context("failed to close DHT records")?;

    // Delete keyring entries
    super::keystore::delete_all_keys().await?;

    // Delete session file
    let session_path = helpers::session_path()?;
    if session_path.exists() {
        std::fs::remove_file(&session_path)
            .context("failed to delete session file")?;
    }

    if mode.is_structured() {
        crate::output::format::print_structured(
            &serde_json::json!({"status": "destroyed"}),
            mode,
        )
    } else {
        format::print_text("Identity destroyed.")
    }
}

/// `rekindle init --wipe-all-data` — factory reset.
pub async fn cmd_wipe(_cfg: &Config, mode: OutputMode) -> anyhow::Result<()> {
    let confirmed = helpers::confirm_destructive(
        "This will delete ALL local Rekindle data including identity, communities, \
         keys, config, and Veilid storage. This cannot be undone.",
        "wipe all data",
    )?;

    if !confirmed {
        format::print_text("Cancelled.")?;
        return Ok(());
    }

    // Delete keyring entries (best-effort)
    let _ = super::keystore::delete_all_keys().await;

    // Delete session
    let session_path = helpers::session_path()?;
    if session_path.exists() {
        let _ = std::fs::remove_file(&session_path);
    }

    // Delete Veilid storage
    let storage = helpers::storage_dir(None)?;
    if storage.exists() {
        let _ = std::fs::remove_dir_all(&storage);
    }

    // Delete config directory (user-level only — never touch /etc)
    if let Ok(config_dir) = helpers::config_dir() {
        if config_dir.exists() {
            let _ = std::fs::remove_dir_all(&config_dir);
        }
    }

    if mode.is_structured() {
        crate::output::format::print_structured(
            &serde_json::json!({"status": "wiped"}),
            mode,
        )
    } else {
        format::print_text("All data wiped. Run `rekindle init` to start fresh.")
    }
}

/// Null handler — used during the init ceremony when we don't need to
/// handle inbound events (we just created the identity, nobody can message us yet).
struct NullHandler;

#[allow(clippy::manual_async_fn)]
impl rekindle_transport::InboundHandler for NullHandler {
    fn on_dm(
        &self,
        _: &rekindle_transport::VerifiedSender,
        _: rekindle_transport::payload::dm::DmPayload,
        _: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    fn on_gossip(
        &self,
        _: &str,
        _: &str,
        _: rekindle_transport::payload::gossip::GossipPayload,
        _: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    fn on_gossip_forward(
        &self,
        _: &rekindle_transport::payload::gossip::SignedGossipEnvelope,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    fn on_voice(
        &self,
        _: &str,
        _: rekindle_transport::payload::voice::VoicePayload,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    fn on_call(
        &self,
        _: Option<&str>,
        _: rekindle_transport::payload::rpc::InboundCall,
    ) -> impl std::future::Future<Output = rekindle_transport::payload::rpc::CallResponse> + Send
    {
        async { rekindle_transport::payload::rpc::CallResponse::Ack }
    }
    fn on_value_change(
        &self,
        _: &str,
        _: Vec<u32>,
        _: Option<Vec<u8>>,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
    fn on_event(
        &self,
        _: rekindle_transport::TransportEvent,
    ) -> impl std::future::Future<Output = ()> + Send {
        async {}
    }
}

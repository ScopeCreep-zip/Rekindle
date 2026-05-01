//! Identity lifecycle commands — init, show, export, import, rotate, destroy.

mod ceremony;
pub(crate) mod export;
pub(crate) mod keystore;

use crate::cli::{ExportCmd, IdentityCmd, ImportCmd, InitArgs};
use crate::config::schema::Config;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Handle `rekindle init`.
pub async fn cmd_init(
    args: &InitArgs,
    cfg: &Config,
    existing_session: Option<&rekindle_transport::Session>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if args.wipe_all_data {
        return ceremony::cmd_wipe(cfg, mode).await;
    }
    ceremony::cmd_init(args, cfg, existing_session, mode).await
}

/// Dispatch `rekindle identity <subcommand>`.
pub async fn dispatch(
    cmd: &IdentityCmd,
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    cfg: &Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        IdentityCmd::Show { json } => {
            let effective_mode = if *json { OutputMode::Json } else { mode };
            cmd_show(session, effective_mode)
        }
        IdentityCmd::Export { path, passphrase } => {
            export::cmd_export(session, path, *passphrase, mode).await
        }
        IdentityCmd::Import { path, passphrase } => {
            export::cmd_import(path, *passphrase, cfg, mode)
        }
        IdentityCmd::Rotate { force } => {
            ceremony::cmd_rotate(handle, session, *force, mode).await
        }
        IdentityCmd::Destroy => ceremony::cmd_destroy(handle, session, mode).await,
    }
}

/// Dispatch `rekindle export <subcommand>`.
pub async fn dispatch_export(
    cmd: &ExportCmd,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ExportCmd::Identity { path } => export::cmd_export(session, path, false, mode).await,
        ExportCmd::Friends { path } => export::cmd_export_friends(session, path, mode),
        ExportCmd::Communities { path } => {
            export::cmd_export_communities(session, path, mode)
        }
    }
}

/// Dispatch `rekindle import <subcommand>`.
pub fn dispatch_import(
    cmd: &ImportCmd,
    cfg: &Config,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        ImportCmd::Identity { path } => export::cmd_import(path, false, cfg, mode),
    }
}

/// `rekindle identity show` — display local identity information.
fn cmd_show(session: &rekindle_transport::Session, mode: OutputMode) -> anyhow::Result<()> {
    if mode.is_structured() {
        return crate::output::format::print_structured(
            &serde_json::json!({
                "public_key": session.identity.public_key_hex,
                "display_name": session.identity.display_name,
                "profile_dht_key": session.identity.profile_dht_key,
                "mailbox_dht_key": session.identity.mailbox_dht_key,
                "friend_list_dht_key": session.identity.friend_list_dht_key,
                "communities": session.communities.len(),
            }),
            mode,
        );
    }

    crate::output::format::print_kv(
        &[
            ("Public key:", session.identity.public_key_hex.clone()),
            ("Display name:", session.identity.display_name.clone()),
            ("Profile DHT:", session.identity.profile_dht_key.clone()),
            ("Mailbox DHT:", session.identity.mailbox_dht_key.clone()),
            (
                "Friend list DHT:",
                session.identity.friend_list_dht_key.clone(),
            ),
            (
                "Communities:",
                session.communities.len().to_string(),
            ),
        ],
        mode,
    )
}

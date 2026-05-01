//! Prekey bundle management — status and replenish.

use anyhow::Context;

use rekindle_transport::operations::mek;
use rekindle_transport::Session;

use crate::cli::PrekeyCmd;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle key prekeys <subcommand>`.
pub async fn dispatch(
    cmd: &PrekeyCmd,
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        PrekeyCmd::Status => cmd_status(handle, session, mode).await,
        PrekeyCmd::Replenish => cmd_replenish(handle, session, mode).await,
    }
}

/// Show prekey availability count.
///
/// Reads the prekey bundle from the profile DHT record and reports
/// how many prekeys are available. The threshold for the doctor `crypto.prekeys.low`
/// check is 10 — below that, replenishment is recommended.
async fn cmd_status(
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let dht = handle
        .node()
        .dht()
        .map_err(|e| anyhow::anyhow!("DHT access: {e}"))?;

    let count = dht
        .profile()
        .prekey_count(&session.identity.profile_dht_key)
        .await
        .context("failed to read prekey count")?;

    let threshold = 10u32;
    let status = if count >= threshold {
        "[OK]"
    } else if count > 0 {
        "[LOW]"
    } else {
        "[EMPTY]"
    };

    if mode.is_structured() {
        return format::print_structured(
            &serde_json::json!({
                "available": count,
                "threshold": threshold,
                "status": status.trim_matches(&['[', ']'][..]),
            }),
            mode,
        );
    }

    format::print_text(&format!("Prekey status: {status}"))?;
    format::print_text(&format!("  Available: {count}"))?;
    format::print_text(&format!("  Threshold: {threshold}"))?;

    if count < threshold {
        format::print_text("")?;
        format::print_text("  Replenish with: rekindle key prekeys replenish")?;
    }

    Ok(())
}

/// Generate and publish fresh prekeys.
async fn cmd_replenish(
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let signing_key = crate::identity::keystore::load_signing_key().await?;

    let bytes_written = mek::replenish_prekeys(
        handle.node(),
        &session.identity.profile_dht_key,
        &signing_key,
    )
    .await
    .context("failed to replenish prekeys")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "replenished",
                "bytes_written": bytes_written,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Prekeys replenished ({bytes_written} bytes published to profile DHT)."
        ))
    }
}

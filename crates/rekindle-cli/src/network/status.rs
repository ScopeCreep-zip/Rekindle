//! Node and network status display commands.

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// `rekindle status` — overview of node health.
pub async fn cmd_status(
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    watch: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if watch {
        return cmd_status_watch(handle, session, mode).await;
    }
    render_status_once(handle, session, mode)
}

/// `rekindle status --watch` — continuous refresh.
async fn cmd_status_watch(
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    loop {
        // Clear screen for fresh render (text mode only)
        if !mode.is_structured() {
            let mut stdout = std::io::stdout();
            use std::io::Write;
            write!(stdout, "\x1b[2J\x1b[H")?;
        }

        // Inline the status display to avoid async recursion
        render_status_once(handle, session, mode)?;

        if mode.is_structured() && mode == OutputMode::Json {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(())
}

/// Render a single status snapshot (used by both one-shot and watch).
fn render_status_once(
    handle: &TransportHandle,
    session: &rekindle_transport::Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let snapshot = handle.node().status_snapshot();

    if mode.is_structured() {
        return format::print_structured(&snapshot, mode);
    }

    format::print_text("Node Status")?;
    format::print_kv(
        &[
            (
                "Attachment:",
                format!(
                    "{} {}",
                    status_glyph(snapshot.is_attached),
                    snapshot.attachment
                ),
            ),
            (
                "Public Internet:",
                if snapshot.public_internet_ready {
                    "ready".into()
                } else {
                    "not ready".into()
                },
            ),
            ("Uptime:", helpers::format_uptime(snapshot.uptime_secs)),
            ("Peers:", snapshot.peer_count.to_string()),
            (
                "Route:",
                if snapshot.route_allocated {
                    match snapshot.route_age_secs {
                        Some(age) => format!("allocated ({})", helpers::format_uptime(age)),
                        None => "allocated".into(),
                    }
                } else {
                    "not allocated".into()
                },
            ),
        ],
        mode,
    )?;

    format::print_text("")?;
    format::print_text("Identity")?;
    format::print_kv(
        &[
            ("Public key:", session.identity.public_key_hex.clone()),
            ("Display name:", session.identity.display_name.clone()),
            (
                "Communities:",
                session.communities.len().to_string(),
            ),
        ],
        mode,
    )
}

/// `rekindle network status` — detailed network state.
pub fn cmd_network_status(
    handle: &TransportHandle,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let snapshot = handle.node().status_snapshot();
    let peer_summary = handle.node().peers().read().circuit_summary();

    if mode.is_structured() {
        return format::print_structured(
            &serde_json::json!({
                "node": snapshot,
                "peers": peer_summary,
            }),
            mode,
        );
    }

    format::print_text("Network Status")?;
    format::print_kv(
        &[
            ("Attachment:", snapshot.attachment.clone()),
            (
                "Attached:",
                snapshot.is_attached.to_string(),
            ),
            (
                "Public Internet:",
                snapshot.public_internet_ready.to_string(),
            ),
            ("Uptime:", helpers::format_uptime(snapshot.uptime_secs)),
        ],
        mode,
    )?;

    format::print_text("")?;
    format::print_text("Peer Summary")?;
    format::print_kv(
        &[
            ("Total:", peer_summary.total.to_string()),
            ("Healthy:", peer_summary.healthy.to_string()),
            ("Degraded:", peer_summary.degraded.to_string()),
            ("Circuit open:", peer_summary.circuit_open.to_string()),
        ],
        mode,
    )
}

/// `rekindle network config` — show safety routing configuration.
pub fn cmd_network_config(
    handle: &TransportHandle,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let config = handle.node().config();

    if mode.is_structured() {
        return format::print_structured(config, mode);
    }

    format::print_text("Safety Routing Configuration")?;
    format::print_text("")?;

    for (name, profile) in [
        ("Text", &config.safety.text),
        ("Voice", &config.safety.voice),
        ("DHT", &config.safety.dht),
        ("RPC", &config.safety.rpc),
    ] {
        format::print_text(&format!("  {name}:"))?;
        crate::output::table::print_kv_table(
            &[
                ("hop_count", profile.hop_count.to_string()),
                ("stability", format!("{:?}", profile.stability)),
                ("sequencing", format!("{:?}", profile.sequencing)),
            ],
            mode,
        )?;
    }

    Ok(())
}

fn status_glyph(attached: bool) -> &'static str {
    if attached { "[ONLINE]" } else { "[OFFLINE]" }
}

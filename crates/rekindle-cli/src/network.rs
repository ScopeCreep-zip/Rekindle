//! Network and status commands.

use rekindle_node::ipc::protocol::IpcRequest;

use crate::cli::NetworkCmd;
use crate::helpers;
use crate::output::{format, table};
use crate::output::OutputMode;
use crate::transport::DaemonClient;

fn dir_size(path: &std::path::Path) -> u64 {
    fn walk(path: &std::path::Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(path) else { return 0 };
        entries.filter_map(std::result::Result::ok).map(|e| {
            let Ok(meta) = e.metadata() else { return 0 };
            if meta.is_file() { meta.len() } else if meta.is_dir() { walk(&e.path()) } else { 0 }
        }).sum()
    }
    walk(path)
}

/// Unified status command — handles compact, --doctor, and --watch.
pub async fn cmd_status(client: &DaemonClient, args: &crate::cli::StatusArgs, mode: OutputMode) -> anyhow::Result<()> {
    use rekindle_types::display::{StatusSnapshot, Check};

    let value = client.request_ok(IpcRequest::Status).await?;

    if mode.is_structured() {
        return format::print_structured(&value, mode);
    }

    let snapshot: StatusSnapshot = serde_json::from_value(value)
        .map_err(|e| anyhow::anyhow!("status parse failed: {e}"))?;

    if args.doctor.is_some() {
        // Expanded: compact status + full diagnostic checks
        print_status_compact(&snapshot, mode)?;
        format::print_text("")?;

        // Append CLI-side local checks
        let mut checks = snapshot.checks;
        let storage_info = helpers::storage_dir(None).map_or("unknown".into(), |p| {
            let size = dir_size(&p);
            format!("{} ({})", p.display(), helpers::format_bytes(size))
        });
        checks.push(Check::pass("local.storage", "local", storage_info));
        checks.push(Check::pass("local.cli_version", "local", env!("CARGO_PKG_VERSION")));

        // Filter by category if specified
        let category_filter = args.doctor.as_deref().filter(|c| *c != "all");
        let filtered: Vec<Check> = if let Some(cat) = category_filter {
            checks.into_iter().filter(|c| c.category == cat).collect()
        } else {
            checks
        };

        format::print_doctor_checks(&filtered, mode, false)?;

        if args.exit_code {
            use rekindle_types::display::CheckStatus;
            let has_fail = filtered.iter().any(|c| c.status == CheckStatus::Fail);
            let has_warn = filtered.iter().any(|c| c.status == CheckStatus::Warn);
            let code = if has_fail { 2 } else { i32::from(has_warn) };
            std::process::exit(code);
        }
    } else {
        // Compact status only
        print_status_compact(&snapshot, mode)?;
    }

    Ok(())
}

/// Render compact status as key-value pairs.
fn print_status_compact(snapshot: &rekindle_types::display::StatusSnapshot, mode: OutputMode) -> anyhow::Result<()> {
    let route = if snapshot.route_allocated {
        format!("allocated ({}s)", snapshot.route_age_secs.unwrap_or(0))
    } else {
        "none".into()
    };

    let pairs = vec![
        ("State", snapshot.state.clone()),
        ("Identity", if snapshot.has_identity {
            snapshot.identity_display_name.clone().unwrap_or_else(|| "initialized".into())
        } else {
            "not initialized".into()
        }),
        ("Attachment", snapshot.attachment.clone()),
        ("Public Internet", snapshot.public_internet_ready.to_string()),
        ("Peers", snapshot.peer_count.to_string()),
        ("Route", route),
        ("Uptime", helpers::format_uptime(snapshot.uptime_secs)),
        ("Communities", snapshot.community_count.to_string()),
        ("Watches", snapshot.active_watches.to_string()),
        ("Gossip Meshes", format!("{} ({} peers)", snapshot.gossip_meshes, snapshot.gossip_mesh_peers)),
    ];
    format::print_kv(&pairs, mode)
}

/// Show status when the daemon is unreachable. Shows only local state.
pub fn cmd_status_offline(mode: OutputMode) -> anyhow::Result<()> {
    let storage_info = helpers::storage_dir(None).map_or("unknown".into(), |p| {
        let size = dir_size(&p);
        format!("{} ({})", p.display(), helpers::format_bytes(size))
    });
    let session_exists = helpers::session_path()
        .map(|p| p.exists())
        .unwrap_or(false);

    if mode.is_structured() {
        let value = serde_json::json!({
            "daemon": "not running",
            "session_exists": session_exists,
            "storage": storage_info,
        });
        return format::print_structured(&value, mode);
    }

    let pairs = vec![
        ("Daemon", "not running".to_string()),
        ("Session file", if session_exists { "exists".into() } else { "not found — run: rekindle init".into() }),
        ("Storage", storage_info),
    ];
    format::print_kv(&pairs, mode)?;
    format::print_text("\n  start the daemon: rekindle node start")
}

pub async fn dispatch(cmd: &NetworkCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        NetworkCmd::Peers { .. } => {
            let value = client.request_ok(IpcRequest::NetworkPeers).await?;
            if mode.is_structured() {
                return format::print_structured(&value, mode);
            }
            let peers = value.as_array().map(|arr| {
                arr.iter().map(|p| vec![
                    p.get("key_short").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    p.get("has_route").and_then(serde_json::Value::as_bool).map_or("?".into(), |b| if b { "yes".into() } else { "no".into() }),
                    p.get("failure_count").and_then(serde_json::Value::as_u64).map_or("0".into(), |n| n.to_string()),
                    p.get("circuit_open").and_then(serde_json::Value::as_bool).map_or("no".into(), |b| if b { "OPEN".into() } else { "closed".into() }),
                ]).collect::<Vec<_>>()
            }).unwrap_or_default();
            table::print_table(&["Peer", "Route", "Failures", "Circuit"], &peers, mode)
        }
        NetworkCmd::Status | NetworkCmd::Routes { .. } | NetworkCmd::Config => {
            let value = client.request_ok(IpcRequest::NetworkStatus).await?;
            format::print_structured(&value, mode)
        }
    }
}

//! Peer listing and circuit breaker display.

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// `rekindle network peers` — list known peers with health state.
pub fn cmd_peers(handle: &TransportHandle, mode: OutputMode) -> anyhow::Result<()> {
    let peers = handle.node().peers().read().snapshot();

    if mode.is_structured() {
        return format::print_structured(&peers, mode);
    }

    if peers.is_empty() {
        format::print_text("No known peers.")?;
        return Ok(());
    }

    let headers = &["Peer", "Route", "Age", "Circuit", "Failures"];
    let rows: Vec<Vec<String>> = peers
        .iter()
        .map(|p| {
            let route_status = if p.has_route {
                "[OK] active".to_string()
            } else {
                "[STALE] expired".to_string()
            };

            let age = helpers::format_uptime(p.route_age_secs);

            let circuit = if p.circuit_open {
                "[OPEN] tripped".to_string()
            } else {
                "[CLOSED] ok".to_string()
            };

            vec![
                p.key_short.clone(),
                route_status,
                age,
                circuit,
                p.failure_count.to_string(),
            ]
        })
        .collect();

    table::print_table(headers, &rows, mode)?;

    // Summary
    let summary = handle.node().peers().read().circuit_summary();
    format::print_text(&format!(
        "\n{} peers: {} healthy, {} degraded, {} circuit open",
        summary.total, summary.healthy, summary.degraded, summary.circuit_open
    ))
}

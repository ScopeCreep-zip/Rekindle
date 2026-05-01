//! Route management commands.

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// `rekindle network routes` — show allocated routes.
pub async fn cmd_routes(
    handle: &TransportHandle,
    refresh: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if refresh {
        format::print_text("Refreshing routes...")?;
        let (route_id, _blob) = handle
            .node()
            .allocate_route()
            .await
            .map_err(|e| anyhow::anyhow!("route refresh failed: {e}"))?;
        format::print_text(&format!("  New route allocated: {route_id}"))?;
    }

    let route_mgr = handle.node().routes();
    let mgr = route_mgr.read();

    if mode.is_structured() {
        return format::print_structured(
            &serde_json::json!({
                "has_route": mgr.has_route(),
                "route_age_secs": mgr.route_age().map(|d| d.as_secs()),
            }),
            mode,
        );
    }

    if mgr.has_route() {
        let age = mgr
            .route_age()
            .map_or_else(|| "unknown".into(), |d| helpers::format_uptime(d.as_secs()));
        format::print_text(&format!("Private route: [OK] allocated (age: {age})"))?;
    } else {
        format::print_text("Private route: [NONE] not allocated")?;
        format::print_text("  allocate with: rekindle network routes --refresh")?;
    }

    Ok(())
}

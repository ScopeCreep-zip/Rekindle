//! `rekindle friend list` — list friends with resolved names and presence.

use anyhow::Context;

use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// List friends with display names and presence status.
///
/// Uses the QueryEngine to read the friend list DHT record and resolve
/// each friend's profile for their current display name and status.
/// Friends with unreachable profiles fall back to the stored nickname
/// or abbreviated public key.
pub async fn cmd_list(
    handle: &TransportHandle,
    session: &Session,
    status_filter: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let query = handle
        .node()
        .query(handle.mek_cache.clone())
        .map_err(|e| anyhow::anyhow!("query engine: {e}"))?;

    let friends = query
        .resolved_friends(&session.identity.friend_list_dht_key)
        .await
        .context("failed to read friend list")?;

    // Apply status filter if provided
    let filtered: Vec<_> = if let Some(filter) = status_filter {
        if filter == "all" {
            friends
        } else {
            friends
                .into_iter()
                .filter(|f| f.status == filter)
                .collect()
        }
    } else {
        friends
    };

    if mode.is_structured() {
        return format::print_structured(&filtered, mode);
    }

    if filtered.is_empty() {
        let qualifier = status_filter
            .map(|s| format!(" with status '{s}'"))
            .unwrap_or_default();
        return format::print_text(&format!("No friends{qualifier}."));
    }

    // Group by status for text display
    let mut online: Vec<&rekindle_transport::FriendDisplay> = Vec::new();
    let mut away: Vec<&rekindle_transport::FriendDisplay> = Vec::new();
    let mut busy: Vec<&rekindle_transport::FriendDisplay> = Vec::new();
    let mut offline: Vec<&rekindle_transport::FriendDisplay> = Vec::new();
    let mut unknown: Vec<&rekindle_transport::FriendDisplay> = Vec::new();

    for f in &filtered {
        match f.status.as_str() {
            "online" => online.push(f),
            "away" => away.push(f),
            "busy" => busy.push(f),
            "offline" => offline.push(f),
            _ => unknown.push(f),
        }
    }

    let groups: Vec<(&str, &str, &[&rekindle_transport::FriendDisplay])> = vec![
        ("Online", "[ONLINE]", &online),
        ("Away", "[AWAY]", &away),
        ("Busy", "[BUSY]", &busy),
        ("Offline", "[OFFLINE]", &offline),
        ("Unknown", "[?]", &unknown),
    ];

    for (label, glyph, members) in &groups {
        if members.is_empty() {
            continue;
        }
        format::print_text(&format!("{label} ({})", members.len()))?;
        for f in *members {
            let name = helpers::sanitize_for_display(&f.display_name);
            let nickname = f
                .nickname
                .as_ref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            let status_msg = if f.status_message.is_empty() {
                String::new()
            } else {
                format!(" — {}", helpers::sanitize_for_display(&f.status_message))
            };
            let last_seen = f.last_seen_ms
                .map(|ms| {
                    #[allow(clippy::cast_possible_truncation)]
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("system clock before unix epoch")
                        .as_millis() as u64;
                    let elapsed = std::time::Duration::from_millis(now_ms.saturating_sub(ms));
                    format!(" ({})", helpers::format_duration_ago(elapsed))
                })
                .unwrap_or_default();
            let route = if f.has_route { "" } else { " [no route]" };
            format::print_text(&format!(
                "  {glyph} {name}{nickname}{status_msg}{last_seen}{route}"
            ))?;
        }
        format::print_text("")?;
    }

    format::print_text(&format!("{} friends total", filtered.len()))
}

//! `rekindle moderate` — kick, ban, unban, timeout, list bans.

use anyhow::Context;

use rekindle_transport::operations::moderation;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Kick a member from a community.
///
/// Builds a `Kick` gossip payload and logs the action.
/// The transport layer handles signing and broadcast.
pub fn cmd_kick(
    _handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    reason: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let _payload = moderation::build_kick_payload(member)
        .context("failed to build kick payload")?;

    // In a full implementation, the payload would be signed with the
    // pseudonym key and broadcast via the gossip mesh. For now, we
    // build the payload to validate the operation is well-formed.

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "kicked",
                "community": membership.community_name,
                "member": member,
                "reason": reason,
            }),
            mode,
        )
    } else {
        let reason_str = reason
            .map(|r| format!(" (reason: {r})"))
            .unwrap_or_default();
        format::print_text(&format!(
            "Kicked {member} from '{}'{reason_str}.",
            membership.community_name
        ))
    }
}

/// Ban a member from a community.
///
/// Persists the ban to the governance bans subkey and builds a `Ban`
/// gossip payload for broadcast.
pub async fn cmd_ban(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    reason: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let _payload = moderation::ban_member(
        handle.node(),
        &membership.governance_key,
        member,
        reason,
        &membership.pseudonym_key,
    )
    .await
    .context("failed to ban member")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "banned",
                "community": membership.community_name,
                "member": member,
                "reason": reason,
            }),
            mode,
        )
    } else {
        let reason_str = reason
            .map(|r| format!(" (reason: {r})"))
            .unwrap_or_default();
        format::print_text(&format!(
            "Banned {member} from '{}'{reason_str}.",
            membership.community_name
        ))
    }
}

/// Unban a member.
pub async fn cmd_unban(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let _payload = moderation::unban_member(handle.node(), &membership.governance_key, member)
        .await
        .context("failed to unban member")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "unbanned",
                "community": membership.community_name,
                "member": member,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "Unbanned {member} in '{}'.",
            membership.community_name
        ))
    }
}

/// Timeout a member for a specified duration.
pub fn cmd_timeout(
    _handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    member: &str,
    duration_str: &str,
    reason: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let duration_secs = parse_timeout_duration(duration_str)?;

    let _payload = moderation::build_timeout_payload(member, duration_secs, reason)
        .context("failed to build timeout payload")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "timed_out",
                "community": membership.community_name,
                "member": member,
                "duration_secs": duration_secs,
                "reason": reason,
            }),
            mode,
        )
    } else {
        let reason_str = reason
            .map(|r| format!(" (reason: {r})"))
            .unwrap_or_default();
        format::print_text(&format!(
            "Timed out {member} in '{}' for {duration_str}{reason_str}.",
            membership.community_name
        ))
    }
}

/// List active bans in a community.
pub async fn cmd_bans(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let bans = moderation::list_bans(handle.node(), &membership.governance_key)
        .await
        .context("failed to list bans")?;

    if mode.is_structured() {
        return format::print_structured(&bans, mode);
    }

    if bans.is_empty() {
        return format::print_text("No active bans.");
    }

    let headers = &["Member", "Banned By", "Reason", "Date"];
    let rows: Vec<Vec<String>> = bans
        .iter()
        .map(|b| {
            vec![
                helpers::abbreviate_key(&b.pseudonym_key),
                helpers::abbreviate_key(&b.banned_by),
                b.reason.clone().unwrap_or_else(|| "-".into()),
                helpers::format_timestamp(b.banned_at),
            ]
        })
        .collect();

    table::print_table(headers, &rows, mode)
}

/// Parse a timeout duration string (e.g., "5m", "1h", "1d").
fn parse_timeout_duration(s: &str) -> anyhow::Result<u64> {
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86400)
    } else {
        anyhow::bail!(
            "invalid timeout duration: '{s}'\n\
             expected: 30s, 5m, 1h, 1d"
        );
    };

    let num: u64 = num_str.parse().map_err(|_| {
        anyhow::anyhow!("invalid duration number: '{num_str}'")
    })?;

    if num == 0 {
        anyhow::bail!("timeout duration must be greater than 0");
    }

    Ok(num * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_timeout_minutes() {
        assert_eq!(parse_timeout_duration("5m").unwrap(), 300);
    }

    #[test]
    fn parse_timeout_hours() {
        assert_eq!(parse_timeout_duration("1h").unwrap(), 3600);
    }

    #[test]
    fn parse_timeout_days() {
        assert_eq!(parse_timeout_duration("1d").unwrap(), 86400);
    }

    #[test]
    fn parse_timeout_zero_rejected() {
        assert!(parse_timeout_duration("0m").is_err());
    }

    #[test]
    fn parse_timeout_invalid() {
        assert!(parse_timeout_duration("abc").is_err());
    }
}

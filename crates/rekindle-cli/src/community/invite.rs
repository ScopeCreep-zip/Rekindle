//! `rekindle community invite` — create, list, and revoke invites.

use anyhow::Context;

use rekindle_transport::operations::invites;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Create an invite code for a community.
pub async fn cmd_invite_create(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    max_uses: Option<u32>,
    expires: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let expires_secs = parse_duration_secs(expires)?;
    let max = max_uses.unwrap_or(0);

    let invite_code = invites::create_invite(
        handle.node(),
        &membership.governance_key,
        &membership.pseudonym_key,
        max,
        expires_secs,
    )
    .await
    .context("failed to create invite")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "created",
                "invite_code": invite_code,
                "community": membership.community_name,
                "max_uses": max,
                "expires_secs": expires_secs,
            }),
            mode,
        )
    } else {
        format::print_text(&format!("Invite created for '{}':", membership.community_name))?;
        format::print_text(&format!("  Code: {invite_code}"))?;
        format::print_text(&format!(
            "  Max uses: {}",
            if max == 0 {
                "unlimited".to_string()
            } else {
                max.to_string()
            }
        ))?;
        format::print_text(&format!(
            "  Expires: {}",
            expires.unwrap_or("never")
        ))?;
        format::print_text("")?;
        format::print_text("Share this code: rekindle community join --invite <code>")
    }
}

/// List active invites for a community.
pub async fn cmd_invite_list(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let active = invites::list_invites(handle.node(), &membership.governance_key)
        .await
        .context("failed to list invites")?;

    if mode.is_structured() {
        return format::print_structured(&active, mode);
    }

    if active.is_empty() {
        return format::print_text("No active invites.");
    }

    let headers = &["Code Hash", "Created By", "Uses", "Max", "Expires"];
    let rows: Vec<Vec<String>> = active
        .iter()
        .map(|inv| {
            vec![
                helpers::abbreviate_key(&inv.code_hash),
                helpers::abbreviate_key(&inv.created_by),
                inv.use_count.to_string(),
                if inv.max_uses == 0 {
                    "∞".to_string()
                } else {
                    inv.max_uses.to_string()
                },
                inv.expires_at
                    .map_or_else(|| "never".into(), helpers::format_timestamp),
            ]
        })
        .collect();

    table::print_table(headers, &rows, mode)
}

/// Revoke an invite by code.
pub async fn cmd_invite_revoke(
    handle: &TransportHandle,
    session: &Session,
    invite_code: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // We need to find which community this invite belongs to.
    // Try all communities the user is in.
    for membership in session.communities.values() {
        if let Ok(()) = invites::revoke_invite(handle.node(), &membership.governance_key, invite_code).await {
            if mode.is_structured() {
                return format::print_structured(
                    &serde_json::json!({
                        "status": "revoked",
                        "community": membership.community_name,
                    }),
                    mode,
                );
            }
            return format::print_text(&format!(
                "Invite revoked in '{}'.",
                membership.community_name
            ));
        }
    }

    anyhow::bail!(
        "invite code not found in any joined community\n\
         the invite may have already been revoked or may belong to a community you haven't joined"
    )
}

/// Parse a human-readable duration string to seconds.
///
/// Supports: "30s", "5m", "1h", "24h", "7d", "never", or None.
fn parse_duration_secs(input: Option<&str>) -> anyhow::Result<Option<u64>> {
    let Some(s) = input else { return Ok(None) };

    if s == "never" {
        return Ok(None);
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('s') {
        (n, 1u64)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86400)
    } else if let Some(n) = s.strip_suffix('w') {
        (n, 604_800)
    } else {
        anyhow::bail!(
            "invalid duration format: '{s}'\n\
             expected: 30s, 5m, 1h, 24h, 7d, 1w, or 'never'"
        );
    };

    let num: u64 = num_str.parse().map_err(|_| {
        anyhow::anyhow!(
            "invalid duration number: '{num_str}'\n\
             expected a positive integer followed by s/m/h/d/w"
        )
    })?;

    Ok(Some(num * multiplier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_seconds() {
        assert_eq!(parse_duration_secs(Some("30s")).unwrap(), Some(30));
    }

    #[test]
    fn parse_duration_minutes() {
        assert_eq!(parse_duration_secs(Some("5m")).unwrap(), Some(300));
    }

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration_secs(Some("24h")).unwrap(), Some(86400));
    }

    #[test]
    fn parse_duration_days() {
        assert_eq!(parse_duration_secs(Some("7d")).unwrap(), Some(604_800));
    }

    #[test]
    fn parse_duration_weeks() {
        assert_eq!(parse_duration_secs(Some("1w")).unwrap(), Some(604_800));
    }

    #[test]
    fn parse_duration_never() {
        assert_eq!(parse_duration_secs(Some("never")).unwrap(), None);
    }

    #[test]
    fn parse_duration_none() {
        assert_eq!(parse_duration_secs(None).unwrap(), None);
    }

    #[test]
    fn parse_duration_invalid() {
        assert!(parse_duration_secs(Some("abc")).is_err());
        assert!(parse_duration_secs(Some("5x")).is_err());
    }
}

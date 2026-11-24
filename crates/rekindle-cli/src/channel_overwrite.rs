//! Channel permission overwrites (`rekindle community channel overwrite`)
//! over the daemon's `ChannelOverwriteSet` / `ChannelOverwriteDelete`. The
//! host validates and reviews each write; the CLI only shapes the request.

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_types::id::RoleId;

use crate::cli::{CommunityChannelCmd, OverwriteCmd};
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use rekindle_client::DaemonClient;

/// `everyone`, `role:<id>` or `member:<pseudonym>` as the request's
/// `(target_type, target_id)`; @everyone is the role `RoleId::EVERYONE`.
pub fn parse_target(s: &str) -> anyhow::Result<(String, String)> {
    if s == "everyone" {
        return Ok(("role".to_owned(), RoleId::EVERYONE.to_hex()));
    }
    if let Some(role) = s.strip_prefix("role:") {
        return Ok(("role".to_owned(), crate::governance::parse_role_id(role)?));
    }
    if let Some(member) = s.strip_prefix("member:").filter(|m| !m.is_empty()) {
        return Ok(("member".to_owned(), member.to_owned()));
    }
    anyhow::bail!("invalid target: {s} (everyone, role:<role id> or member:<pseudonym>)")
}

/// The union of every given permission bitmask.
fn mask(perms: &[String]) -> anyhow::Result<u64> {
    perms.iter().try_fold(0u64, |acc, p| {
        Ok(acc | crate::governance::parse_permissions(p)?)
    })
}

/// The daemon request `cmd` sends.
pub fn request(cmd: &OverwriteCmd) -> anyhow::Result<IpcRequest> {
    let channel = match cmd {
        OverwriteCmd::Set {
            community,
            channel,
            target,
            allow,
            deny,
        } => {
            let (target_type, target_id) = parse_target(target)?;
            ChannelRequest::ChannelOverwriteSet {
                community: community.clone(),
                channel_id: channel.clone(),
                target_type,
                target_id,
                allow: mask(allow)?,
                deny: mask(deny)?,
            }
        }
        OverwriteCmd::Delete {
            community,
            channel,
            target,
        } => {
            let (target_type, target_id) = parse_target(target)?;
            ChannelRequest::ChannelOverwriteDelete {
                community: community.clone(),
                channel_id: channel.clone(),
                target_type,
                target_id,
            }
        }
    };
    Ok(IpcRequest::Community(CommunityRequest::Channels(channel)))
}

pub async fn dispatch(
    cmd: &CommunityChannelCmd,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let CommunityChannelCmd::Overwrite(cmd) = cmd;
    let value = helpers::send(client, request(cmd)?).await?;
    format::print_structured(&value, mode)
}

#[cfg(test)]
#[path = "channel_overwrite_tests.rs"]
mod tests;

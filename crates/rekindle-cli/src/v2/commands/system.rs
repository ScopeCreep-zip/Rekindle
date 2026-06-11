//! System/operator commands: announcements, raid alerts, lockdown, kick notify, bootstrap, sync.

use crate::v2::prelude::DaemonRequest;

use crate::v2::cli::SystemCmd;
use crate::v2::helpers;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::prelude::DaemonClient;

pub async fn dispatch(cmd: &SystemCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        SystemCmd::Announce { community, body } => {
            let value = client.request_ok(DaemonRequest::SystemAnnounce {
                community: community.clone(),
                body: body.clone(),
            }).await?;
            helpers::audit_log("announce", community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::RaidAlert { community, active } => {
            let value = client.request_ok(DaemonRequest::RaidAlert {
                community: community.clone(),
                active: *active,
            }).await?;
            let action = if *active { "raid_alert_activate" } else { "raid_alert_deactivate" };
            helpers::audit_log(action, community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::Lockdown { community, locked } => {
            let value = client.request_ok(DaemonRequest::LockdownToggle {
                community: community.clone(),
                locked: *locked,
            }).await?;
            let action = if *locked { "lockdown_activate" } else { "lockdown_deactivate" };
            helpers::audit_log(action, community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::KickNotify { community, target } => {
            let value = client.request_ok(DaemonRequest::KickNotify {
                community: community.clone(),
                target_pseudonym: target.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::BootstrapRequest { community } => {
            let value = client.request_ok(DaemonRequest::BootstrapRequest {
                community: community.clone(),
            }).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::SyncRequest { community, channel_id, since } => {
            let value = client.request_ok(DaemonRequest::SyncRequest {
                community: community.clone(),
                channel_id: channel_id.clone(),
                since_timestamp: *since,
            }).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::PolicyReload => {
            let value = client.request_ok(DaemonRequest::PolicyReload).await?;
            format::print_structured(&value, mode)
        }
    }
}

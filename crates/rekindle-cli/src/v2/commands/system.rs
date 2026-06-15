//! System/operator commands: announcements, raid alerts, lockdown, kick notify, bootstrap, sync.

use crate::v2::prelude::{ChatRequest, DaemonRequest, LifecycleRequest};

use crate::v2::cli::SystemCmd;
use crate::v2::helpers;
use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::prelude::DaemonClient;

pub async fn dispatch(cmd: &SystemCmd, client: &DaemonClient, mode: OutputMode) -> anyhow::Result<()> {
    match cmd {
        SystemCmd::Announce { community, body } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::SystemAnnounce {
                community: community.clone(),
                body: body.clone(),
            })).await?;
            helpers::audit_log("announce", community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::RaidAlert { community, active } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::RaidAlert {
                community: community.clone(),
                active: *active,
            })).await?;
            let action = if *active { "raid_alert_activate" } else { "raid_alert_deactivate" };
            helpers::audit_log(action, community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::Lockdown { community, locked } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::LockdownToggle {
                community: community.clone(),
                locked: *locked,
            })).await?;
            let action = if *locked { "lockdown_activate" } else { "lockdown_deactivate" };
            helpers::audit_log(action, community, "ok");
            format::print_structured(&value, mode)
        }
        SystemCmd::KickNotify { community, target } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::KickNotify {
                community: community.clone(),
                target_pseudonym: target.clone(),
            })).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::BootstrapRequest { community } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::BootstrapRequest {
                community: community.clone(),
            })).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::SyncRequest { community, channel_id, since } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::SyncRequest {
                community: community.clone(),
                channel_id: channel_id.clone(),
                since_timestamp: *since,
            })).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::PolicyReload => {
            let value = client.request_ok(DaemonRequest::Lifecycle(LifecycleRequest::PolicyReload)).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::AuditLog { community, limit } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::AuditLog {
                community: community.clone(),
                limit: *limit,
            })).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::OnboardingConfig { community } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::OnboardingConfigGet {
                community: community.clone(),
            })).await?;
            format::print_structured(&value, mode)
        }
        SystemCmd::WelcomeScreen { community } => {
            let value = client.request_ok(DaemonRequest::Chat(ChatRequest::WelcomeScreenGet {
                community: community.clone(),
            })).await?;
            format::print_structured(&value, mode)
        }
    }
}

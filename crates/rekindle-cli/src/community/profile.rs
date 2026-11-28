//! Readable CLI arguments become typed per-community settings requests.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, SettingsRequest};
use rekindle_types::dto::community_profile::ProfileField;

use crate::{cli::ProfileArgs, helpers, output::format, output::OutputMode};

pub(super) async fn set_name(
    client: &DaemonClient,
    mode: OutputMode,
    community: &str,
    name: Option<&str>,
    clear: bool,
) -> anyhow::Result<()> {
    let display_name = match (name, clear) {
        (Some(name), false) => ProfileField::Set(name.to_owned()),
        (None, true) => ProfileField::Clear,
        _ => anyhow::bail!("give a name or --clear"),
    };
    settings(
        client,
        mode,
        SettingsRequest::CommunitySetDisplayName {
            community: community.to_owned(),
            display_name,
        },
    )
    .await
}

pub(super) async fn update(
    client: &DaemonClient,
    mode: OutputMode,
    args: &ProfileArgs,
) -> anyhow::Result<()> {
    settings(
        client,
        mode,
        SettingsRequest::CommunityUpdateProfile {
            community: args.community.clone(),
            patch: args.patch()?,
        },
    )
    .await
}

async fn settings(
    client: &DaemonClient,
    mode: OutputMode,
    request: SettingsRequest,
) -> anyhow::Result<()> {
    let value = helpers::send(
        client,
        IpcRequest::Community(CommunityRequest::Settings(request)),
    )
    .await?;
    format::print_structured(&value, mode)
}

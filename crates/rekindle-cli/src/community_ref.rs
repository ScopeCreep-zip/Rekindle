//! Resolve terminal community and channel references against the Host's lists.

use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_types::display::{CommunityDetail, CommunityOverview};
use rekindle_utils::text::sanitize_for_display;

use crate::helpers;

/// List joined communities in the Host's wire format.
pub async fn list(client: &DaemonClient) -> anyhow::Result<serde_json::Value> {
    helpers::send(
        client,
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::CommunityList)),
    )
    .await
}

pub async fn anchor(client: &DaemonClient, given: &str) -> anyhow::Result<String> {
    let communities: Vec<CommunityOverview> = serde_json::from_value(list(client).await?)?;
    let name = given.to_lowercase();
    Ok(pick(
        communities
            .iter()
            .map(|c| (c.anchor.as_str(), c.name.as_str())),
        given,
        "community",
        |candidate| candidate.to_lowercase() == name,
    )?
    .to_owned())
}

pub async fn detail(client: &DaemonClient, given: &str) -> anyhow::Result<CommunityDetail> {
    let anchor = anchor(client, given).await?;
    let value = helpers::send(
        client,
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityInfo { anchor },
        )),
    )
    .await?;
    Ok(serde_json::from_value(value)?)
}

pub fn channel_id<'a>(detail: &'a CommunityDetail, given: &str) -> anyhow::Result<&'a str> {
    pick(
        detail
            .channels
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str())),
        given,
        "channel",
        |name| name == given,
    )
}

/// Exact IDs take precedence over names. List every matching name with
/// its full ID so the user can select one on the next invocation.
fn pick<'a>(
    candidates: impl Iterator<Item = (&'a str, &'a str)> + Clone,
    given: &str,
    entity: &str,
    matches_name: impl Fn(&str) -> bool,
) -> anyhow::Result<&'a str> {
    if let Some((id, _)) = candidates.clone().find(|(id, _)| *id == given) {
        return Ok(id);
    }
    let mut matches: Vec<_> = candidates.filter(|(_, name)| matches_name(name)).collect();
    match matches.as_slice() {
        [(id, _)] => Ok(id),
        [] => anyhow::bail!("no {entity} named \"{}\"", sanitize_for_display(given)),
        _ => {
            matches.sort_unstable();
            let choices: Vec<_> = matches
                .iter()
                .map(|(id, name)| format!("{} ({id})", sanitize_for_display(name)))
                .collect();
            anyhow::bail!(
                "ambiguous {entity} \"{}\": {}; give the ID",
                sanitize_for_display(given),
                choices.join(", ")
            )
        }
    }
}

#[cfg(test)]
#[path = "community_ref_tests.rs"]
mod tests;

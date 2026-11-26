//! A community window's pack buttons (plan E7.7 HSC1: every Xfire button
//! works in every layout; CR1: a refused button is drawn disabled with the
//! Host's reason). The composed community window (SKW2) draws the pack's
//! chat-frame row (Symbiosis's Call, Mute, Join and Profile); its shell
//! announces `SkinSelection::Community` on open and on every channel
//! change, and the Host maps that row here:
//!
//! - `VoiceChat`/`HostVoiceChat` join the selected channel when it is a
//!   voice channel, else the community's first voice channel. The shell's
//!   `join_voice_channel` holds community voice until plan E4.1 makes
//!   `VoiceJoin` the Host's ("exists, now held"), so the press is
//!   `CallOp::JoinVoice`, run by the shell as every `CallOp` is now.
//! - `MuteSelf`/`UnmuteSelf` are `CallOp::Mute`/`Unmute`.
//! - `JoinGameLobby` joins the community's game server (plan E3.1 H1,
//!   `HostGovernance::game_servers`, the list the community window's
//!   "SERVERS (n)" sidebar shows): the selected server, else the first
//!   listed, launched by the one launch path a friend's Join takes
//!   (`IpcRequest::GameLaunch`, `JoinSource::Community`, TrustChanging for
//!   a peer-chosen address), judged by the Host's one launch check
//!   (`journal_entry::not_joinable`, as the info page's server rows are).
//!   Refused only when the community lists no server, or the server
//!   cannot be launched.
//! - `ViewProfile`/`ProfileFriend` open the community's info panel
//!   (`ShellOp::OpenCommunityInfo`).

use std::collections::BTreeMap;

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::{Capability, SkinAction as A};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun, SkinSelection};
use rekindle_types::friend::NotJoinableReason;
use rekindle_types::game_server::{GameServerInfo, JoinSource, ServerAddress};
use rekindle_types::subscription_events::VoiceScope;

use super::actions::Mapped;
use super::signals::Inputs;
use crate::daemon::dispatch::{adapter, DaemonContext};
use crate::host::community::{ChannelType, CommunityState};

/// One of a community's game servers as the skin reads it: its id, its
/// game, its address and the Host's launch verdict on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommunityServer {
    pub id: String,
    pub game_id: Option<u32>,
    pub address: String,
    pub not_joinable: Option<NotJoinableReason>,
}

impl CommunityServer {
    /// `s` judged by the Host's one launch check (the info page's server
    /// rows, `infoview::compose::server_row`).
    fn of(s: GameServerInfo) -> Self {
        let game_id = s.game_id.parse().ok();
        Self {
            not_joinable: crate::host::journal_entry::not_joinable(game_id, Some(&s.address)),
            id: s.id,
            game_id,
            address: s.address,
        }
    }
}

/// A joined community as the skin reads it: its voice channels, in the
/// community's channel order, and its game servers, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SkinCommunity {
    pub voice: Vec<String>,
    pub servers: Vec<CommunityServer>,
}

impl SkinCommunity {
    fn of(state: &CommunityState) -> Self {
        Self {
            voice: state
                .channels
                .iter()
                .filter(|c| matches!(c.channel_type, ChannelType::Voice))
                .map(|c| c.id.clone())
                .collect(),
            servers: Vec::new(),
        }
    }
}

/// The joined communities (only `only` when given) with their voice
/// channels and game servers. A community whose servers we may not view
/// (`PermissionDenied`, as the info page reads it) or could not read
/// lists none here, so its Join is refused, never launched blind.
pub(crate) async fn load(
    ctx: &DaemonContext,
    only: Option<&str>,
) -> BTreeMap<String, SkinCommunity> {
    let mut views: BTreeMap<String, SkinCommunity> = ctx
        .communities
        .map()
        .read()
        .iter()
        .filter(|(id, _)| only.is_none_or(|o| o == id.as_str()))
        .map(|(id, c)| (id.clone(), SkinCommunity::of(c)))
        .collect();
    let host = adapter(ctx);
    for (id, view) in &mut views {
        match host.game_servers(id).await {
            Ok(servers) => view.servers = servers.into_iter().map(CommunityServer::of).collect(),
            Err(GovernanceRuntimeError::PermissionDenied) => {}
            Err(e) => tracing::warn!(community = %id, error = %e, "skin: game servers not read"),
        }
    }
    views
}

/// What a community window's press resolves against: the selected
/// community, its view (`None` when we hold no such community: not a
/// member), and the channel the window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommunityRead {
    pub community: String,
    pub view: Option<SkinCommunity>,
    pub channel: Option<String>,
    pub server: Option<String>,
}

/// [`CommunityRead`] for a window whose selection is a community; `view`
/// looks a community up among the joined ones.
pub(crate) fn read(
    selection: Option<&SkinSelection>,
    view: impl Fn(&str) -> Option<SkinCommunity>,
) -> Option<CommunityRead> {
    let Some(SkinSelection::Community {
        community,
        channel,
        server,
    }) = selection
    else {
        return None;
    };
    Some(CommunityRead {
        community: community.clone(),
        view: view(community),
        channel: channel.clone(),
        server: server.clone(),
    })
}

pub(crate) const NOT_MEMBER: &str = "You're not a member of this community.";
pub(crate) const NO_VOICE: &str = "This community has no voice channel.";
pub(crate) const IN_VOICE: &str = "You're already in this voice channel. Press Leave to hang up.";
pub(crate) const NOT_IN_VOICE: &str = "You're not in this community's voice channel.";
pub(crate) const NO_SERVER: &str = "This community lists no game server to join.";
const NOT_IN_CALL: &str = "You're not in a call or a voice channel.";

/// The voice channel Call joins: the selected channel when it is a voice
/// channel, else the community's first.
fn voice_target<'a>(view: &'a SkinCommunity, channel: Option<&str>) -> Option<&'a str> {
    channel
        .and_then(|c| view.voice.iter().find(|v| *v == c))
        .or_else(|| view.voice.first())
        .map(String::as_str)
}

/// The channel Call joins, or why it cannot.
fn join_target<'a>(
    view: Option<&'a SkinCommunity>,
    channel: Option<&str>,
) -> Result<&'a str, &'static str> {
    let view = view.ok_or(NOT_MEMBER)?;
    voice_target(view, channel).ok_or(NO_VOICE)
}

/// The server Join launches to (the selected server, else the first
/// listed), or why it cannot: not a member, no server listed, or the
/// Host's launch check refuses it.
fn server_target<'a>(
    view: Option<&'a SkinCommunity>,
    server: Option<&str>,
) -> Result<(u32, &'a str), String> {
    let view = view.ok_or(NOT_MEMBER)?;
    let target = server
        .and_then(|id| view.servers.iter().find(|s| s.id == id))
        .or_else(|| view.servers.first())
        .ok_or(NO_SERVER)?;
    match (target.game_id, target.not_joinable) {
        (Some(game_id), None) => Ok((game_id, &target.address)),
        (_, reason) => Err(format!(
            "This community's server {} cannot be joined: {:?}.",
            target.address,
            reason.unwrap_or(NotJoinableReason::CannotLaunch)
        )),
    }
}

/// A community window's verdict on one capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommunityVerdict {
    Holds,
    /// Refused, with the Host's words.
    Refused(String),
}

impl CommunityVerdict {
    /// The refusal's words; `None` while it holds.
    pub(crate) fn words(&self) -> Option<&str> {
        match self {
            Self::Holds => None,
            Self::Refused(why) => Some(why),
        }
    }
}

/// A community window's verdict on `capability`: `None` when the
/// community scope does not decide it (the general table does).
pub(crate) fn verdict(
    capability: Capability,
    inputs: &Inputs,
    community: &str,
    view: Option<&SkinCommunity>,
    channel: Option<&str>,
    server: Option<&str>,
) -> Option<CommunityVerdict> {
    use Capability as C;
    let in_voice_here = |c: Option<&str>| match inputs.voice() {
        Some(VoiceScope::Community {
            community: joined,
            channel: on,
        }) => joined == community && c.is_none_or(|c| on == c),
        _ => false,
    };
    let in_any = inputs.in_call() || inputs.voice().is_some();
    let words = match capability {
        C::VoiceChat | C::HostVoiceChat => match join_target(view, channel) {
            Err(why) => Some(why),
            Ok(target) if in_voice_here(Some(target)) => Some(IN_VOICE),
            Ok(_) => None,
        },
        C::JoinGameLobby => {
            let words = server_target(view, server).err();
            return Some(words.map_or(CommunityVerdict::Holds, CommunityVerdict::Refused));
        }
        C::LeaveVoiceChat | C::Profile if view.is_none() => Some(NOT_MEMBER),
        C::LeaveVoiceChat => (!in_voice_here(None)).then_some(NOT_IN_VOICE),
        C::CancelVoiceChat => Some("There's no outgoing call ringing to cancel."),
        C::MuteSelf | C::UnmuteSelf if !in_any => Some(NOT_IN_CALL),
        C::MuteSelf => inputs.self_muted().then_some("You're already muted."),
        C::UnmuteSelf => (!inputs.self_muted()).then_some("You're not muted."),
        C::Profile => None,
        _ => return None,
    };
    Some(words.map_or(CommunityVerdict::Holds, |w| {
        CommunityVerdict::Refused(w.to_owned())
    }))
}

/// A community window's press of `action`: `None` for an action the
/// community scope does not decide (the general table maps it).
pub(crate) fn map(action: &A, read: &CommunityRead) -> Option<Result<Mapped, IpcResponse>> {
    let shell = |op| Ok(Mapped::Run(SkinRun::Shell(op)));
    let refused = |why: &str| Err(IpcResponse::error(409, why.to_owned()));
    let view = read.view.as_ref();
    Some(match action {
        A::VoiceChat | A::HostVoiceChat => match join_target(view, read.channel.as_deref()) {
            Ok(channel) => shell(ShellOp::Call(CallOp::JoinVoice {
                community: read.community.clone(),
                channel: channel.to_owned(),
            })),
            Err(why) => refused(why),
        },
        A::MuteSelf => shell(ShellOp::Call(CallOp::Mute)),
        A::UnmuteSelf => shell(ShellOp::Call(CallOp::Unmute)),
        A::JoinGameLobby => match server_target(view, read.server.as_deref()) {
            Ok((game_id, address)) => match ServerAddress::parse(address) {
                Ok(server) => Ok(Mapped::Request(Box::new(IpcRequest::GameLaunch {
                    game_id,
                    server: Some(server),
                    source: JoinSource::Community,
                }))),
                Err(e) => refused(&format!("This community's server {address}: {e}")),
            },
            Err(why) => refused(&why),
        },
        A::ViewProfile | A::ProfileFriend if view.is_none() => refused(NOT_MEMBER),
        A::ViewProfile | A::ProfileFriend => shell(ShellOp::OpenCommunityInfo {
            community: read.community.clone(),
        }),
        _ => return None,
    })
}

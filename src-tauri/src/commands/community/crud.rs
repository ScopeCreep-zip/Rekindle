use rekindle_ipc::protocol::LifecycleRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;
use rekindle_types::dto::community::{
    CoCreationSnapshot, CommunityCreated, CommunityJoined, CommunityView,
};

host_command! {
    state;
    /// The joined communities (`IpcRequest::Community(CommunityRequest::CommunityList)`).
    pub async fn get_communities(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<rekindle_types::display::CommunityOverview>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(LifecycleRequest::CommunityList))
    }
}

host_command! {
    state;
    /// Every joined community as its window renders it
    /// (`IpcRequest::Community(CommunityRequest::CommunityViews)`).
    pub async fn get_community_details(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<CommunityView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityViews,
        ))
    }
}

host_command! {
    state;
    /// The held co-creator invites and our pending creations' co-creator
    /// states (`LifecycleRequest::CoCreationSnapshot`, plan E3.2 S7s.3g), read
    /// once the window's event stream is live.
    pub async fn get_co_creation(
        state: State<'_, SharedState>,
    ) -> Result<Done<CoCreationSnapshot>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CoCreationSnapshot,
        ))
    }
}

host_command! {
    state;
    /// Create a community (`IpcRequest::Community(CommunityRequest::CommunityCreate)`).
    ///
    /// `approval_required` picks the genesis admission mode and is
    /// immutable afterwards: the CRDT honours `AdmissionPolicy` only as the
    /// genesis entry, so a later one is ignored by every honest peer.
    pub async fn create_community(
        name: String,
        approval_required: Option<bool>,
        co_creators: Option<Vec<String>>,
        state: State<'_, SharedState>,
    ) -> Result<Done<CommunityCreated>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name,
                description: String::new(),
                approval_required: approval_required.unwrap_or(false),
                co_creators: co_creators.unwrap_or_default(),
            },
        ))
    }
}

host_command! {
    state;
    /// Stop waiting on a co-creator of our pending creation that has not
    /// answered (`LifecycleRequest::CreatorRemove`, plan E3.2 S7s.3).
    pub async fn remove_co_creator(
        community_id: String,
        identity: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CreatorRemove {
                anchor: community_id,
                identity,
            },
        ))
    }
}

host_command! {
    state;
    /// Publish a successor of a community we created
    /// (`LifecycleRequest::CommunityPublishSuccessor`, plan E3.2 S7s.4).
    pub async fn publish_community_successor(
        community_id: String,
        continuing: Option<Vec<String>>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityPublishSuccessor {
                anchor: community_id,
                continuing,
            },
        ))
    }
}

host_command! {
    state;
    /// Follow an offered successor with the invite sealed to us in its invites
    /// record (`LifecycleRequest::CommunityFollowSuccessor`, plan E3.2 S7s.4,
    /// lead decision (34)).
    pub async fn follow_community_successor(
        community_id: String,
        new_anchor: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityFollowSuccessor {
                anchor: community_id,
                new_anchor,
            },
        ))
    }
}

host_command! {
    state;
    /// The successors offered for a community this session
    /// (`LifecycleRequest::CommunitySuccessors`, lead decision (33)).
    pub async fn get_community_successors(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunitySuccessors {
                anchor: community_id,
            },
        ))
    }
}

host_command! {
    state;
    /// Cancel our pending creation (`LifecycleRequest::CommunityCreateCancel`).
    pub async fn cancel_community_create(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreateCancel {
                anchor: community_id,
            },
        ))
    }
}

host_command! {
    state;
    /// Accept or decline a held co-creator invite
    /// (`LifecycleRequest::CreatorInviteAnswer`).
    pub async fn answer_creator_invite(
        community_id: String,
        accept: bool,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CreatorInviteAnswer {
                anchor: community_id,
                accept,
            },
        ))
    }
}

host_command! {
    state;
    /// Join a community from its invite link
    /// (`IpcRequest::Community(CommunityRequest::CommunityJoin)`), reviewed with
    /// the fingerprint of the invite's record key.
    pub async fn join_community(
        invite_url: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<CommunityJoined>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: invite_url },
        ))
    }
}

host_command! {
    state;
    /// Leave a community (`IpcRequest::Community(CommunityRequest::CommunityLeave)`), reviewed with its
    /// leave escalations.
    pub async fn leave_community(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityLeave {
                anchor: community_id,
            },
        ))
    }
}

host_command! {
    state;
    /// Enrol this device as a fresh device of our membership
    /// (`LifecycleRequest::CommunityDeviceEnrol`, ADR 0022 rule 1, lead decision
    /// 63): the re-enrol action a `NeedsReenrol` device status offers, reviewed
    /// as a trust-changing act.
    pub async fn community_device_enrol(
        community_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        device_enrol_request(community_id)
    }
}

/// The request `community_device_enrol` sends.
fn device_enrol_request(community_id: String) -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityDeviceEnrol {
            anchor: community_id,
        },
    ))
}

host_command! {
    state;
    /// Edit the community's name, description, icon or banner; `None` keeps
    /// the merged value (`IpcRequest::Community(CommunityRequest::CommunityUpdateInfo)`).
    pub async fn update_community_info(
        community_id: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityUpdateInfo {
                anchor: community_id,
                name,
                description,
                icon_hash,
                banner_hash,
            },
        ))
    }
}

#[cfg(test)]
#[path = "crud_tests.rs"]
mod tests;

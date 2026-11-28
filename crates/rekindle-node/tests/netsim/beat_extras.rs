//! Plan E3.4 S8-pre, the lead's ruling on `58eb04bd7`: our beat's sealed
//! extras come from the Host's own state, our game (`host::own_presence`)
//! through our sharing consent (`set_presence_policy`), never from a
//! shell's copy. The creator plays a game: under the default consent its
//! beat seals nothing, and once it shares its activity its next beat
//! carries the extras sealed. Node 1 reads the creator's beat as
//! `beat_engine` does.

use rekindle_codec::member_record::BeatState;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, SettingsRequest};
use rekindle_types::presence::{PresenceSharingPolicy, SessionStatus, ShareScope};

use crate::beat_engine::{reads_state, set_status, AT_ONCE};
use crate::harness::Community;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_beat_seals_extras_as_the_hosts_sharing_consent_lets_them_out() {
    let c = Community::of(2, 5541).await;
    c.nodes[0]
        .ok(IpcRequest::GamePresenceSet {
            game_name: "CS2".into(),
            game_id: Some(7),
            elapsed_seconds: 0,
            server_address: None,
        })
        .await;
    set_status(&c, 0, SessionStatus::Busy).await;
    let withheld = reads_state(&c, BeatState::Busy, AT_ONCE).await;
    assert!(
        withheld.extras.is_none(),
        "under the default consent the creator's beat seals no extras"
    );

    c.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Settings(
            SettingsRequest::CommunitySetPresencePolicy {
                community: c.anchor.clone(),
                policy: PresenceSharingPolicy {
                    share_activity: ShareScope::Members,
                    ..PresenceSharingPolicy::default()
                },
            },
        )))
        .await;
    set_status(&c, 0, SessionStatus::Online).await;
    let shared = reads_state(&c, BeatState::Online, AT_ONCE).await;
    assert!(
        shared.extras.is_some(),
        "once the creator's Host consents to share its activity, its beat carries the game sealed"
    );
}

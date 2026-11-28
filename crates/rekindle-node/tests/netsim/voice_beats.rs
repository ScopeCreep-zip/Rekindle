//! Both Host modes publish held voice sessions into the beat view.

use std::sync::Arc;
use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_node::host::{Host, ShellHooks};

use crate::beat_engine::AT_ONCE;
use crate::community_voice::{join, voice_channel};
use crate::harness::{eventually, start_with, Community};
use crate::shell_node::{BareShell, SimMode};

/// Below the model's fastest DHT set. The request answers while the beat is pending.
const ANSWERS_BEFORE_A_SET: Duration = Duration::from_millis(300);

fn claim(host: &Host, community: &str, member: &str, channel: Option<&str>) -> bool {
    host.netsim_voice_claims(community).iter().any(|row| {
        row.pseudonym_hex == member
            && row.voice_channel_id.as_deref() == channel
            && (channel.is_none() || row.fresh)
    })
}

async fn join_and_leave_from_beats(seed: u64, joining_mode: SimMode) {
    let (sim, nodes) = start_with(3, seed, &|i| {
        let mode = match i {
            0 => SimMode::Daemon,
            1 => joining_mode,
            _ => SimMode::Shell,
        };
        let hooks =
            (mode == SimMode::Shell).then(|| Arc::new(BareShell::default()) as Arc<dyn ShellHooks>);
        (mode, hooks)
    })
    .await;
    let mut community = Community::create_on((sim, nodes)).await;
    community.join(1).await;
    community.join(2).await;
    let channel = voice_channel(&community).await;
    let anchor = &community.anchor;
    let member = community.pseudonym(1).await;
    let sender = &community.nodes[1];
    let observer = &community.nodes[2].host;

    let asked = tokio::time::Instant::now();
    let joined = sender.ok(join(anchor, &channel)).await;
    assert_eq!(joined["channel"], channel);
    assert!(
        asked.elapsed() < ANSWERS_BEFORE_A_SET,
        "join answers while the beat write is pending"
    );
    assert!(
        eventually(&community.sim, AT_ONCE, || async {
            claim(observer, anchor, &member, Some(&channel))
        })
        .await,
        "the peer's beat view receives the held voice claim"
    );

    let asked = tokio::time::Instant::now();
    let left = sender.ok(IpcRequest::VoiceLeave).await;
    assert_eq!(left["channel"], channel);
    assert!(
        asked.elapsed() < ANSWERS_BEFORE_A_SET,
        "leave answers while the beat write is pending"
    );
    assert!(
        eventually(&community.sim, AT_ONCE, || async {
            claim(observer, anchor, &member, None)
        })
        .await,
        "the peer's beat view clears the claim"
    );
    assert!(sender.call(IpcRequest::VoiceLeave).await.is_err());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_voice_join_and_leave_reach_a_peer_roster_from_beats_alone() {
    join_and_leave_from_beats(5531, SimMode::Shell).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_daemon_members_voice_join_and_leave_reach_a_shell_roster_at_once() {
    join_and_leave_from_beats(5537, SimMode::Daemon).await;
}

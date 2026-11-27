//! Plan E3.4 S8-pre on the shell mode: a member's voice join reaches a
//! peer's voice roster from beats alone. The netsim's shells run no
//! registry poll (`BareShell::spawn_presence_poll_tick` and
//! `presence_refresh` do nothing), so every roster they are handed comes
//! from the beat view (`view::reconcile_voice`). The creator runs on the
//! daemon mode: a shell-mode node answers no bootstrap request in the
//! netsim (`shell_node.rs`: that answer is `src-tauri`'s until F1), so the
//! two shell members join through it, as `shell_member_route` does.
//!
//! A daemon member's voice session is the Host's own (`host::own_voice`):
//! its `VoiceJoin` and `VoiceLeave` write its beat at once, so a shell
//! member's roster lists it and drops it well inside one scheduled beat.

use std::sync::Arc;
use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_node::host::ShellHooks;

use crate::beat_engine::AT_ONCE;
use crate::harness::{eventually, start_with, Community};
use crate::shell_node::{BareShell, SimMode};

const VOICE: &str = "0123456789abcdef0123456789abcdef";

/// Whether the last roster `shell` was handed for `community` puts
/// `member` live in `channel` (`None`: in no channel).
fn last_claim(shell: &BareShell, community: &str, member: &str, channel: Option<&str>) -> bool {
    (shell.session.rosters.lock().iter().rev())
        .find(|(c, _)| c == community)
        .is_some_and(|(_, rows)| {
            rows.iter().any(|r| {
                r.pseudonym_hex == member
                    && r.voice_channel_id.as_deref() == channel
                    && (channel.is_none() || r.fresh)
            })
        })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_voice_join_and_leave_reach_a_peer_roster_from_beats_alone() {
    let shells: Vec<Arc<BareShell>> = (0..3).map(|_| Arc::new(BareShell::default())).collect();
    let nodes = start_with(3, 5531, &|i| {
        if i == 0 {
            return (SimMode::Daemon, None);
        }
        let shell = Arc::clone(&shells[i]) as Arc<dyn ShellHooks>;
        (SimMode::Shell, Some(shell))
    })
    .await;
    let mut community = Community::create_on(nodes).await;
    community.join(1).await;
    community.join(2).await;
    let anchor = community.anchor.clone();
    let member = community.pseudonym(1).await;

    shells[1]
        .session
        .voice
        .lock()
        .insert(anchor.clone(), VOICE.to_owned());
    community.nodes[1]
        .host
        .voice_presence_changed(&anchor)
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            last_claim(&shells[2], &anchor, &member, Some(VOICE))
        })
        .await,
        "the peer's voice roster shows the member in the channel from its beat"
    );

    shells[1].session.voice.lock().clear();
    community.nodes[1]
        .host
        .voice_presence_changed(&anchor)
        .await;
    assert!(
        eventually(&community.sim, Duration::from_secs(120), || async {
            last_claim(&shells[2], &anchor, &member, None)
        })
        .await,
        "the member's leave beat takes it out of the peer's voice roster"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_daemon_members_voice_join_and_leave_reach_a_shell_roster_at_once() {
    let shell = Arc::new(BareShell::default());
    let nodes = start_with(3, 5537, &|i| {
        if i < 2 {
            return (SimMode::Daemon, None);
        }
        (
            SimMode::Shell,
            Some(Arc::clone(&shell) as Arc<dyn ShellHooks>),
        )
    })
    .await;
    let mut community = Community::create_on(nodes).await;
    community.join(1).await;
    community.join(2).await;
    let anchor = community.anchor.clone();
    let member = community.pseudonym(1).await;

    let joined = community.nodes[1]
        .ok(IpcRequest::VoiceJoin {
            community: anchor.clone(),
            channel: VOICE.to_owned(),
            muted: false,
            deafened: false,
        })
        .await;
    assert_eq!(joined["channel"], VOICE, "the daemon joined the channel");
    assert!(
        eventually(&community.sim, AT_ONCE, || async {
            last_claim(&shell, &anchor, &member, Some(VOICE))
        })
        .await,
        "the join's own beat puts the daemon member in the shell's voice roster at once"
    );

    let left = community.nodes[1].ok(IpcRequest::VoiceLeave).await;
    assert_eq!(
        left["channel"], VOICE,
        "the daemon left the channel it was in"
    );
    assert!(
        eventually(&community.sim, AT_ONCE, || async {
            last_claim(&shell, &anchor, &member, None)
        })
        .await,
        "the leave's own beat takes the daemon member out of the shell's voice roster"
    );
    assert!(
        community.nodes[1]
            .call(IpcRequest::VoiceLeave)
            .await
            .is_err(),
        "a second leave finds no voice session"
    );
}

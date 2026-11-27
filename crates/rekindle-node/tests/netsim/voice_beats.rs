//! Plan E3.4 S8-pre on the shell mode: a member's voice join reaches a
//! peer's voice roster from beats alone. The netsim's shells run no
//! registry poll (`BareShell::spawn_presence_poll_tick` and
//! `presence_refresh` do nothing), so every roster they are handed comes
//! from the beat view (`view::reconcile_voice`).

use std::sync::Arc;
use std::time::Duration;

use rekindle_node::host::ShellHooks;

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
    let shells: Vec<Arc<BareShell>> = (0..2).map(|_| Arc::new(BareShell::default())).collect();
    let nodes = start_with(2, 5531, &|i| {
        let shell = Arc::clone(&shells[i]) as Arc<dyn ShellHooks>;
        (SimMode::Shell, Some(shell))
    })
    .await;
    let mut community = Community::create_on(nodes).await;
    community.join(1).await;
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
            last_claim(&shells[0], &anchor, &member, Some(VOICE))
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
            last_claim(&shells[0], &anchor, &member, None)
        })
        .await,
        "the member's leave beat takes it out of the peer's voice roster"
    );
}

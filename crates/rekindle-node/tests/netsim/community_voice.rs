//! A held community voice session on three production-pool Hosts.

use std::time::Duration;

use rekindle_ipc::protocol::{ChannelRequest, CommunityRequest, IpcRequest};
use rekindle_types::audio::{AudioIo, VoiceSink, VoiceSource};
use rekindle_types::subscription_events::{JoinHandshake, VoiceScope};

use crate::harness::{eventually, start_with_audio, Community, SimNode};
use crate::shell_node::SimMode;

pub(super) fn join(community: &str, channel: &str) -> IpcRequest {
    IpcRequest::VoiceJoin {
        community: community.to_owned(),
        channel: channel.to_owned(),
        muted: false,
        deafened: false,
    }
}

pub(super) async fn voice_channel(community: &Community) -> String {
    let created = community.nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Channels(
            ChannelRequest::ChannelCreate {
                community: community.anchor.clone(),
                name: "Voice".into(),
                kind: "voice".into(),
                category: None,
                topic: None,
                slowmode_seconds: 0,
                parent_voice_channel: None,
            },
        )))
        .await;
    let id = created["id"].as_str().expect("voice channel id").to_owned();
    assert!(
        eventually(&community.sim, Duration::from_secs(150), || async {
            for node in &community.nodes {
                let channels = node
                    .ok(IpcRequest::Community(CommunityRequest::Channels(
                        ChannelRequest::ChannelList {
                            community: community.anchor.clone(),
                        },
                    )))
                    .await;
                if !channels
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| row["id"] == id))
                {
                    return false;
                }
            }
            true
        })
        .await,
        "the created voice channel reaches every member"
    );
    id
}

// The simulator's pump keeps delivering packets while a call is active.
async fn during_call<F, Fut>(limit: Duration, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    check().await
}

async fn verify_rosters(community: &Community, channel: &str, pseudonyms: &[String]) {
    let anchor = &community.anchor;
    let converged = during_call(Duration::from_secs(150), || async {
        for (index, node) in community.nodes.iter().enumerate() {
            let voice = node.host.voice();
            let Some(roster) = voice.roster().await else {
                return false;
            };
            if pseudonyms
                .iter()
                .enumerate()
                .any(|(peer, pseudonym)| peer != index && !roster.peers.contains(pseudonym))
            {
                return false;
            }
            assert_eq!(roster.peer_count(), 2, "a Host never adds itself as a peer");
            if voice
                .channel_state(anchor, channel)
                .is_none_or(|state| state.handshake() != JoinHandshake::Connected)
            {
                return false;
            }
            let keys = voice.channel_keys(anchor, channel);
            if pseudonyms
                .iter()
                .enumerate()
                .any(|(peer, key)| peer != index && keys.session_key_of(key).is_none())
            {
                return false;
            }
        }
        true
    })
    .await;
    assert!(
        converged,
        "all three engines learn both peers' routes and signed session keys"
    );
}

async fn verify_media(community: &Community, channel: &str, pseudonyms: &[String]) {
    let anchor = &community.anchor;
    assert!(
        during_call(Duration::from_secs(40), || async {
            for (index, node) in community.nodes.iter().enumerate() {
                let voice = node.host.voice();
                let key = voice
                    .channel_keys(anchor, channel)
                    .send_key(voice.clock().monotonic());
                if community.nodes.iter().enumerate().any(|(peer, receiver)| {
                    peer != index
                        && receiver
                            .host
                            .voice()
                            .channel_keys(anchor, channel)
                            .secret_at(&pseudonyms[index], key.index)
                            .map_or(true, |installed| *installed != *key.secret)
                }) {
                    return false;
                }
            }
            true
        })
        .await,
        "both peers install each sender's media key through the bare call path"
    );
    assert!(
        during_call(Duration::from_secs(12), || async {
            for (index, node) in community.nodes.iter().enumerate() {
                if pseudonyms.iter().enumerate().any(|(peer, pseudonym)| {
                    peer != index && audio_sequence(node, pseudonym).is_none()
                }) {
                    return false;
                }
            }
            true
        })
        .await,
        "each Host receives signed voice media from its two peers"
    );

    let muted = &community.nodes[0];
    let receiver = &community.nodes[2];
    let state = muted.ok(IpcRequest::VoiceSetMute { muted: true }).await;
    assert_eq!(state["muted"], true);
    let state = muted
        .ok(IpcRequest::VoiceSetDeafen { deafened: true })
        .await;
    assert_eq!(state["deafened"], true);
    assert_eq!(muted.host.voice().controls(), Some((true, true)));
    // The per-peer driver still has packets already queued before mute.
    tokio::time::sleep(Duration::from_secs(8)).await;
    let muted_sequence = audio_sequence(receiver, &pseudonyms[0]);
    let speaking_sequence =
        audio_sequence(receiver, &pseudonyms[1]).expect("the other peer sent audio");
    assert!(
        during_call(Duration::from_secs(10), || async {
            audio_sequence(receiver, &pseudonyms[1])
                .is_some_and(|sequence| sequence >= speaking_sequence + 30)
        })
        .await,
        "the receiver continues getting audio from the unmuted peer"
    );
    assert_eq!(
        audio_sequence(receiver, &pseudonyms[0]),
        muted_sequence,
        "mute stops audio arriving at the peer"
    );
}

fn audio_sequence(node: &SimNode, sender: &str) -> Option<u64> {
    node.host.voice().media()?.arrivals().audio_sequence(sender)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn community_voice_join_leave() {
    let audio = AudioIo {
        source: VoiceSource::Speech,
        sink: VoiceSink::Null,
    };
    let mut community =
        Community::create_on(start_with_audio(3, 541, &|_| (SimMode::Daemon, None), audio).await)
            .await;
    for index in 1..3 {
        community.join(index).await;
    }
    let channel = voice_channel(&community).await;
    let anchor = &community.anchor;
    let scope = VoiceScope::Community {
        community: anchor.clone(),
        channel: channel.clone(),
    };
    let mut pseudonyms = Vec::new();
    for (index, node) in community.nodes.iter().enumerate() {
        pseudonyms.push(community.pseudonym(index).await);
        let joined = node.ok(join(anchor, &channel)).await;
        assert_eq!(joined["channel"], channel);
        assert_eq!(
            node.host.voice().session(),
            Some(scope.clone()),
            "the Host keeps the channel engine alive"
        );
    }

    verify_rosters(&community, &channel, &pseudonyms).await;
    verify_media(&community, &channel, &pseudonyms).await;

    let voice = community.nodes[0].host.voice();
    let original = voice
        .channel_state(anchor, &channel)
        .expect("the held session");
    community.nodes[0].ok(join("Netsim", &channel)).await;
    assert!(std::sync::Arc::ptr_eq(
        &original,
        &voice.channel_state(anchor, &channel).unwrap()
    ));
    let alternate = if channel.starts_with('0') { "1" } else { "0" };
    let another = format!("{alternate}{}", &channel[1..]);
    let refused = community.nodes[0]
        .call(join(anchor, &another))
        .await
        .expect_err("another channel is refused");
    assert_eq!(refused.message, "Leave voice in Netsim first.");
    assert_eq!(voice.session(), Some(scope));

    let response = community.nodes[0].ok(IpcRequest::VoiceLeave).await;
    assert_eq!(response["channel"], channel);
    assert!(
        voice.session().is_none(),
        "leave tears the live engine down"
    );
    assert!(community.nodes[0]
        .call(IpcRequest::VoiceLeave)
        .await
        .is_err());
    assert!(
        during_call(Duration::from_secs(30), || async {
            for node in &community.nodes[1..] {
                let Some(roster) = node.host.voice().roster().await else {
                    return false;
                };
                if roster.peers.contains(&pseudonyms[0]) {
                    return false;
                }
            }
            true
        })
        .await,
        "the other two engines remove the leaver from their roster"
    );

    community.nodes[1].ok(IpcRequest::Lock).await;
    assert!(
        community.nodes[1].host.voice().session().is_none(),
        "lock tears down the voice engine"
    );
}

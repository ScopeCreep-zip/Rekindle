use super::*;
use crate::subscription_events::{CallEvent, NotificationEvent, VoiceEvent};

#[test]
fn direct_messages_scope_by_peer_or_record() {
    let one_to_one =
        SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
            peer_key: "pk".into(),
            timestamp: 1,
            sender_name: None,
            body: None,
            decryption_failed: false,
            automod_blurred: false,
            conversation_id: "pk".into(),
            server_message_id: None,
            reply_to_id: None,
            is_own: false,
        });
    assert_eq!(one_to_one.scope(), EventScope::Peer("pk".into()));
    let group = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DirectMessageReceived {
        peer_key: "pk".into(),
        timestamp: 1,
        sender_name: None,
        body: None,
        decryption_failed: false,
        automod_blurred: false,
        conversation_id: "VLD0:rec".into(),
        server_message_id: None,
        reply_to_id: None,
        is_own: false,
    });
    assert_eq!(group.scope(), EventScope::Conversation("VLD0:rec".into()));
    let ack = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmSendStatus {
        client_id: "m".into(),
        conversation_id: "pk".into(),
        state: crate::message::SendState::Sent,
        retry: None,
    });
    assert_eq!(ack.scope(), EventScope::Peer("pk".into()));
    let gap = SubscriptionEvent::ChannelMessage(ChannelMessageEvent::DmHistoryGap {
        peer_key: "pk".into(),
        through_seq: 7,
    });
    assert_eq!(gap.scope(), EventScope::Peer("pk".into()));
}

#[test]
fn joins_calls_voice_and_notifications() {
    let join = SubscriptionEvent::Membership(MembershipEvent::MembersRefreshed {
        community: "c".into(),
    });
    assert_eq!(join.scope(), EventScope::Community("c".into()));
    let call = SubscriptionEvent::Call(CallEvent::Ringing {
        call_id: "id".into(),
    });
    assert_eq!(call.scope(), EventScope::Call("id".into()));
    let dm_voice = SubscriptionEvent::Voice(VoiceEvent::LocalJoined {
        scope: VoiceScope::Dm {
            peer_key: "pk".into(),
        },
        join_handshake: crate::subscription_events::JoinHandshake::Announced,
        media_ready: crate::subscription_events::VoiceMediaReady {
            ready: false,
            reason: "handshake-announced".into(),
        },
    });
    assert_eq!(dm_voice.scope(), EventScope::Peer("pk".into()));
    let alert = SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
        id: "a".into(),
        title: "t".into(),
        subject: None,
        body: "b".into(),
    });
    assert_eq!(alert.scope(), EventScope::Device);
    let nick = SubscriptionEvent::Friend(FriendEvent::NicknameChanged {
        peer_key: "pk".into(),
        nickname: None,
    });
    assert_eq!(nick.scope(), EventScope::Peer("pk".into()));
}

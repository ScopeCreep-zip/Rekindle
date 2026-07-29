//! DATAGRAM_PUBLISH handler — topic-filtered delivery with conditions evaluation.

use crate::v4::codec::channel::ack as ack_codec;
use crate::v4::codec::datagram::publish as codec;
use crate::v4::conditions::evaluator::{evaluate, EvalContext};
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::handlers::HandlerError;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

fn wall_clock_secs_since_midnight() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (now % 86400) as u32
}

pub fn handle(
    subscriptions: &SubscriptionRegistry,
    agreed_clearance: Clearance,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    let publish = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    if subscriptions.count() == 0 {
        // Client mode: deliver unconditionally.
        router.on_publish(
            info,
            uuid::Uuid::nil(),
            &publish.topic_hash,
            publish.event_seq,
            &publish.application_payload,
        );
    } else {
        // Server mode: match against transport-level subscriptions.
        let matching_subs = subscriptions.subscriptions_for_topic(&publish.topic_hash);

        let eval_ctx = EvalContext {
            sender_peer_id: &info.peer_id,
            sender_clearance: agreed_clearance,
            frame_body_len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
            topic_hash: Some(&publish.topic_hash),
            transfer_id: None,
            chunk_index: None,
            wall_clock_secs_since_midnight: wall_clock_secs_since_midnight(),
            event_priority: None,
        };

        for sub_id in &matching_subs {
            let conditions = subscriptions.conditions(sub_id);
            let passes = match conditions {
                Some(cond) if !cond.is_empty() => {
                    evaluate(cond, &eval_ctx, 16).unwrap_or(false)
                }
                _ => true,
            };

            if passes {
                router.on_publish(
                    info, *sub_id, &publish.topic_hash,
                    publish.event_seq, &publish.application_payload,
                );
            }
        }
    }

    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(publish.message_id),
    });

    Ok(())
}

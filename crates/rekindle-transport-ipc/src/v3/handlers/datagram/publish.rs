use crate::v3::codec::channel::ack as ack_codec;
use crate::v3::codec::datagram::publish as codec;
use crate::v3::conditions::evaluator::{evaluate, EvalContext};
use crate::v3::context::{OutboundFrame, OutboundChannelKind, SessionContext};
use crate::v3::handlers::HandlerError;

fn wall_clock_secs_since_midnight() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    (now % 86400) as u32
}

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let publish = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let matching_subs = ctx.subscriptions_for_topic(&publish.topic_hash);

    let eval_ctx = EvalContext {
        sender_peer_id: ctx.remote_peer_id(),
        sender_clearance: ctx.agreed_clearance(),
        frame_body_len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
        topic_hash: Some(&publish.topic_hash),
        transfer_id: None,
        chunk_index: None,
        wall_clock_secs_since_midnight: wall_clock_secs_since_midnight(),
        event_priority: None,
    };

    for sub_id in &matching_subs {
        let conditions = ctx.subscription_conditions(sub_id);
        let passes = match conditions {
            Some(cond) if !cond.is_empty() => {
                evaluate(cond, &eval_ctx, 16).unwrap_or(false)
            }
            _ => true,
        };

        if passes {
            let info = ctx.connection_info().clone();
            ctx.router().on_publish(
                &info,
                *sub_id,
                &publish.topic_hash,
                publish.event_seq,
                &publish.application_payload,
            );
        }
    }

    ctx.push_outbound(OutboundFrame::Channel {
        kind: OutboundChannelKind::Ack,
        payload: ack_codec::encode_single(publish.message_id),
    });

    Ok(())
}

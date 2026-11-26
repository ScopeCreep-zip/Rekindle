//! The bare-envelope calls' dispatch (`bare_envelope` says why each is
//! admitted unframed): each hands its request to the handler and replies
//! in the dialect its sender reads. Split from `dispatch` (plan E4.1 S3).

use std::sync::Arc;

use rekindle_records::port::{AppCallId, AppMessagePort};
use tracing::{debug, warn};

use crate::handler::InboundHandler;
use crate::operations::direct::{CALL_DEADLINE, NAK};

/// Hand a bare MEK transfer to the handler and reply in the same
/// dialect it arrived in.
///
/// The reply is the Cap'n Proto `MekTransferAck` the handler built (only
/// the handler holds the unwrapped key that keys its confirmation tag,
/// review S7m part 1), not a serialized `CallResponse`, because the sender
/// awaits exactly that ack and settles the transfer only on a tag that
/// verifies.
pub(super) async fn dispatch_bare_mek_transfer<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    transfer: rekindle_codec::community::envelope::MekTransferPayload,
) {
    let community_id = transfer.community_id.clone();
    // The sender is authenticated by the ECDH wrap, not by an envelope
    // signature, so it is passed through as the claimed sender and the
    // handler's unwrap is what actually decides.
    let sender = transfer.sender_pseudonym.clone();

    let response_bytes = match tokio::time::timeout(
        CALL_DEADLINE,
        handler.on_call(
            Some(sender.as_str()),
            crate::payload::rpc::InboundCall::CommunityMekTransfer(transfer),
        ),
    )
    .await
    {
        // `Ok(bytes)` is the encoded, tagged `MekTransferAck`.
        Ok(crate::payload::rpc::CallResponse::Ok(ack)) => ack,
        Ok(other) => {
            warn!(
                community = %community_id,
                response = ?other,
                "MEK transfer not accepted"
            );
            NAK.to_vec()
        }
        Err(_) => {
            warn!(
                community = %community_id,
                deadline_secs = CALL_DEADLINE.as_secs(),
                "MEK transfer handler exceeded deadline — sending NAK"
            );
            NAK.to_vec()
        }
    };

    if let Err(e) = messages.reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send MEK transfer reply");
    }
}

/// The reply a requester of attachment chunks reads when we hold none of
/// them: the desktop track's answer, so both shells read alike.
const NOTHING_HELD: &[u8] = b"ACK";

/// Hand an admitted bare media key or key request (plan E4.1 S3) to the
/// handler and reply in the desktop's dialect
/// (`voice_signaling_adapter::answer_media_key_call`): the encoded
/// `VoiceMediaKeyAck` envelope the handler built, `ACK` when it has nothing
/// to return, and a NAK when it refused or ran past the deadline.
pub(super) async fn dispatch_bare_media_key<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use crate::payload::rpc::{CallResponse, InboundCall};
    let request = InboundCall::VoiceMediaKey(payload);
    let reply = match tokio::time::timeout(CALL_DEADLINE, handler.on_call(None, request)).await {
        Ok(CallResponse::Ok(ack)) => ack,
        Ok(CallResponse::Ack) => NOTHING_HELD.to_vec(),
        Ok(CallResponse::Rejected { reason }) => {
            debug!(%reason, "media-key call refused");
            NAK.to_vec()
        }
        Err(_) => {
            warn!("media-key call exceeded the call deadline — sending NAK");
            NAK.to_vec()
        }
    };
    if let Err(e) = messages.reply(call_id, reply).await {
        warn!(error = %e, "failed to send media-key reply");
    }
}

/// Hand a bare attachment request to the handler and reply in its
/// dialect: the encoded `MultiAttachmentChunk` envelope the handler
/// built, [`NOTHING_HELD`] when it holds none of the chunks, and a NAK
/// when it refused or ran past the deadline.
pub(super) async fn dispatch_bare_attachment_request<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    request: crate::payload::rpc::InboundCall,
) {
    use crate::payload::rpc::CallResponse;
    let reply = match tokio::time::timeout(CALL_DEADLINE, handler.on_call(None, request)).await {
        Ok(CallResponse::Ok(chunks)) => chunks,
        Ok(CallResponse::Ack) => NOTHING_HELD.to_vec(),
        Ok(CallResponse::Rejected { reason }) => {
            warn!(%reason, "attachment request refused");
            NAK.to_vec()
        }
        Err(_) => {
            warn!(
                deadline_secs = CALL_DEADLINE.as_secs(),
                "attachment request exceeded deadline — sending NAK"
            );
            NAK.to_vec()
        }
    };
    if let Err(e) = messages.reply(call_id, reply).await {
        warn!(error = %e, "failed to send attachment reply");
    }
}

/// Answer a joiner's bare `BootstrapRequest` (plan E3.2) with the
/// handler's encoded `bootstrapResponseV2`, or a member's bare
/// `CheckpointStateRequest` (S7c.3) with its `checkpointStateResponse` (a
/// `CheckpointHintRequest`, S7c.5, likewise). A refusal is a NAK; a handler
/// past [`CALL_DEADLINE`] is answered
/// [`RETRY_LATER`](crate::operations::direct::RETRY_LATER), as a 1:1 call
/// is: a deadline is no refusal (review RV5 H1), and a NAK here withdraws a
/// joiner's join (`rekindle_governance_runtime::bootstrap_peers`). The
/// handler runs on the session scope ([`InboundHandler::scope`]), so its
/// work (a bootstrap answer's invite revoke and key delivery) goes on to
/// completion past the deadline, as `operations::direct::answer_call`'s
/// does (plan JP1); with no session the request is answered retry-later.
pub(super) async fn dispatch_bare_request<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    request: crate::payload::rpc::InboundCall,
) {
    let (answer, answered) = tokio::sync::oneshot::channel();
    let worker = Arc::clone(handler);
    handler
        .scope()
        .spawn_or_drop("answer bare request", async move {
            // Past the deadline the answer has no taker; the work is done.
            let _ = answer.send(worker.on_call(None, request).await);
        });
    let reply = match tokio::time::timeout(CALL_DEADLINE, answered).await {
        Ok(Ok(crate::payload::rpc::CallResponse::Ok(bytes))) => bytes,
        Ok(Ok(other)) => {
            warn!(response = ?other, "bare request refused");
            NAK.to_vec()
        }
        Ok(Err(_)) => {
            debug!("bare request dropped with its session: answered retry-later");
            crate::operations::direct::RETRY_LATER.to_vec()
        }
        Err(_) => {
            warn!("bare request exceeded the call deadline: answered retry-later");
            crate::operations::direct::RETRY_LATER.to_vec()
        }
    };
    if let Err(e) = messages.reply(call_id, reply).await {
        warn!(error = %e, "failed to send bare request reply");
    }
}

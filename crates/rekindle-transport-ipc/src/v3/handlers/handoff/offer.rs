use crate::v3::codec::handoff::accept as accept_codec;
use crate::v3::codec::handoff::offer as offer_codec;
use crate::v3::codec::handoff::reject as reject_codec;
use crate::v3::context::{OutboundFailureCode, OutboundFrame, OutboundHandoffKind, SessionContext};
use crate::v3::handlers::HandlerError;

pub fn handle(ctx: &mut SessionContext, payload: &[u8]) -> Result<(), HandlerError> {
    let offer = offer_codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let handoff_key = ctx.keys().handoff;

    if !offer_codec::verify_mac(payload, &handoff_key) {
        let reject = reject_codec::HandoffRejectPayload {
            reason_code: OutboundFailureCode::HandoffMacFailed,
            handoff_id: offer.handoff_id,
            detail: String::new(),
        };
        ctx.push_outbound(OutboundFrame::Handoff {
            kind: OutboundHandoffKind::Reject,
            payload: reject_codec::encode(&reject),
        });
        ctx.fallback_tracker_mut().record_failure();
        return Err(HandlerError::CodecFailed("handoff MAC verification failed".into()));
    }

    // On Linux with a coordinator: recv the memfd fd via sidechannel,
    // mmap + verify content hash, then accept or reject based on
    // verification result.
    #[cfg(target_os = "linux")]
    if let Some(coord) = ctx.handoff_coordinator_mut() {
        use crate::v3::handoff::coordinator::HandoffOfferInfo;
        let offer_info = HandoffOfferInfo {
            handoff_id: offer.handoff_id,
            stream_id: offer.stream_id,
            content_hash: offer.content_hash,
            handoff_mac: {
                let mut mac = [0u8; 32];
                if payload.len() >= 96 {
                    mac[..16].copy_from_slice(&payload[64..80]);
                }
                mac
            },
            payload_id: 0, // assigned by transport on recv
        };
        match coord.receive_offer(&offer_info) {
            Ok(accept_info) => {
                let accept = accept_codec::HandoffAcceptPayload {
                    handoff_id: offer.handoff_id,
                    accept_wall_ns: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0),
                    verified_content_hash: accept_info.verified_content_hash,
                };
                ctx.push_outbound(OutboundFrame::Handoff {
                    kind: OutboundHandoffKind::Accept,
                    payload: accept_codec::encode(&accept),
                });
                ctx.fallback_tracker_mut().record_success();
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(handoff_id = %offer.handoff_id, error = ?e,
                    "handoff coordinator rejected offer — falling back to socket bulk");
                let reject = reject_codec::HandoffRejectPayload {
                    reason_code: OutboundFailureCode::HandoffMacFailed,
                    handoff_id: offer.handoff_id,
                    detail: format!("{e:?}"),
                };
                ctx.push_outbound(OutboundFrame::Handoff {
                    kind: OutboundHandoffKind::Reject,
                    payload: reject_codec::encode(&reject),
                });
                ctx.fallback_tracker_mut().record_failure();
                return Ok(());
            }
        }
    }

    // Non-Linux or no coordinator: accept after MAC verification only.
    // Content verification is not possible without memfd.
    let accept = accept_codec::HandoffAcceptPayload {
        handoff_id: offer.handoff_id,
        accept_wall_ns: 0,
        verified_content_hash: offer.content_hash,
    };
    ctx.push_outbound(OutboundFrame::Handoff {
        kind: OutboundHandoffKind::Accept,
        payload: accept_codec::encode(&accept),
    });
    ctx.fallback_tracker_mut().record_success();

    Ok(())
}

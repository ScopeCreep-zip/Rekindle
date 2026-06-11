//! Outbound operations — request/reply, bulk transfer, subscription.
//!
//! Control-plane operations (send_request, send_notify, send_goodbye)
//! go through outbound_tx → control loop → lane_channels → write task.
//!
//! Bulk data (send_bulk) uses BulkSender which dispatches STREAM_PAYLOAD
//! chunks directly to the rayon pool. Rayon workers encode and push wire
//! bytes directly to the write task via bulk_wire_tx, bypassing the
//! control loop entirely. Only STREAM_OPEN and STREAM_FIN go through
//! the control loop for lifecycle tracking.
//!
//! Handoff fast path (Linux only): payloads >= handoff_threshold attempt
//! memfd zero-copy transfer before falling back to socket bulk. The
//! handoff decision uses an AtomicBool signal from the control loop's
//! FallbackTracker (Release write, Relaxed read).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::v3::bulk::send::BulkSender;
use crate::v3::codec::channel::goodbye as goodbye_codec;
use crate::v3::codec::datagram::request as request_codec;
use crate::v3::codec::datagram::notify as notify_codec;
use crate::v3::codec::handoff::offer as offer_codec;
use crate::v3::context::OutboundFrame;
use crate::v3::wire::clearance::Clearance;

use super::types::{
    ClientError, SendDelivered, SendError, BulkDelivered, BulkError, ClientPhase,
    ReplyPayload, ReplySender, RequestReplyError,
};

/// Pending ack waiter — resolved when CHANNEL_ACK or DATAGRAM_REJECT arrives.
pub(crate) struct AckWaiter {
    pub tx: oneshot::Sender<Result<(), SendError>>,
}

/// Pending bulk waiter — resolved when STREAM_ACK, NACK, HANDOFF_ACCEPT, or HANDOFF_REJECT arrives.
pub(crate) struct BulkWaiter {
    pub tx: oneshot::Sender<Result<BulkDelivered, BulkError>>,
    pub started_at: Instant,
}

/// Outbound sender state. Held by IpcClient, accessed via `&self`.
pub(crate) struct SendState {
    pub outbound_tx: tokio::sync::mpsc::Sender<OutboundFrame>,
    pub sequenced_tx: tokio::sync::mpsc::Sender<crate::v3::context::SequencedOutbound>,
    pub bulk_sender: BulkSender,
    pub pending_acks: Arc<Mutex<HashMap<uuid::Uuid, AckWaiter>>>,
    pub pending_bulk: Arc<Mutex<HashMap<u8, BulkWaiter>>>,
    pub pending_replies: Arc<Mutex<HashMap<uuid::Uuid, ReplySender>>>,
    pub phase: Arc<Mutex<ClientPhase>>,
    pub agreed_clearance: Clearance,
    /// Payload size threshold for memfd handoff (bytes).
    pub handoff_threshold: u64,
    /// Written by control loop FallbackTracker (Release).
    /// Read by send_bulk (Relaxed — stale reads cause fallback, not failure).
    pub handoff_enabled: Arc<std::sync::atomic::AtomicBool>,
}

impl SendState {
    /// Send a DATAGRAM_REQUEST with ack tracking and timeout.
    pub async fn send_request(
        &self,
        payload: &[u8],
        ack_timeout: Duration,
    ) -> Result<SendDelivered, SendError> {
        if !self.phase.lock().can_send() {
            return Err(SendError::ConnectionNotActive);
        }

        let message_id = uuid::Uuid::now_v7();
        let encoded = request_codec::encode(&request_codec::DatagramRequestPayload {
            message_id,
            reply_timeout_ms: u32::try_from(ack_timeout.as_millis()).unwrap_or(u32::MAX),
            sender_clearance: self.agreed_clearance,
            application_payload: payload.to_vec(),
            conditions: vec![],
        });

        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_acks.lock().insert(message_id, AckWaiter { tx: ack_tx });

        if let Err(e) = self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v3::wire::frame_kind::DatagramKind::Request,
            payload: encoded,
        }).await {
            self.pending_acks.lock().remove(&message_id);
            return Err(SendError::WriteFailed(format!("{e:?}")));
        }

        match tokio::time::timeout(ack_timeout, ack_rx).await {
            Ok(Ok(Ok(()))) => Ok(SendDelivered),
            Ok(Ok(Err(send_err))) => Err(send_err),
            Ok(Err(_)) => Err(SendError::WriteFailed("channel closed".into())),
            Err(_) => {
                self.pending_acks.lock().remove(&message_id);
                Err(SendError::AckTimeout)
            }
        }
    }

    /// Send a DATAGRAM_REQUEST and await the reply payload directly.
    ///
    /// Registers a per-request oneshot in `pending_replies`. The
    /// `ClientRouter::on_reply` callback delivers the payload to this
    /// oneshot instead of pushing to the shared `inbound_tx` channel.
    /// Concurrent callers each get their own oneshot — no serialization,
    /// no `recv()` race.
    ///
    /// Returns the application-level reply payload bytes. The caller
    /// deserializes these with their own codec (e.g., postcard for
    /// `DaemonResponse`).
    pub async fn request_reply(
        &self,
        payload: &[u8],
        reply_timeout: Duration,
    ) -> Result<ReplyPayload, RequestReplyError> {
        if !self.phase.lock().can_send() {
            return Err(RequestReplyError::ConnectionNotActive);
        }

        let message_id = uuid::Uuid::now_v7();
        tracing::debug!(
            %message_id,
            payload_len = payload.len(),
            timeout_ms = reply_timeout.as_millis() as u64,
            "request_reply: sending request",
        );

        let encoded = request_codec::encode(&request_codec::DatagramRequestPayload {
            message_id,
            reply_timeout_ms: u32::try_from(reply_timeout.as_millis()).unwrap_or(u32::MAX),
            sender_clearance: self.agreed_clearance,
            application_payload: payload.to_vec(),
            conditions: vec![],
        });

        let (reply_tx, reply_rx) = oneshot::channel();
        self.pending_replies.lock().insert(message_id, reply_tx);

        if let Err(e) = self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v3::wire::frame_kind::DatagramKind::Request,
            payload: encoded,
        }).await {
            self.pending_replies.lock().remove(&message_id);
            tracing::error!(%message_id, error = %e, "request_reply: outbound channel failed");
            return Err(RequestReplyError::WriteFailed(format!("{e:?}")));
        }

        match tokio::time::timeout(reply_timeout, reply_rx).await {
            Ok(Ok(result)) => {
                tracing::debug!(%message_id, "request_reply: reply received");
                result
            }
            Ok(Err(_)) => {
                tracing::warn!(%message_id, "request_reply: reply channel closed — connection lost");
                Err(RequestReplyError::ChannelClosed)
            }
            Err(_) => {
                self.pending_replies.lock().remove(&message_id);
                tracing::warn!(
                    %message_id,
                    timeout_ms = reply_timeout.as_millis() as u64,
                    "request_reply: timeout — no reply received",
                );
                Err(RequestReplyError::Timeout)
            }
        }
    }

    /// Send a DATAGRAM_NOTIFY (fire-and-forget at application level).
    pub async fn send_notify(&self, payload: &[u8]) -> Result<(), ClientError> {
        let message_id = uuid::Uuid::now_v7();
        let encoded = notify_codec::encode(&notify_codec::DatagramNotifyPayload {
            message_id,
            sender_clearance: self.agreed_clearance,
            application_payload: payload.to_vec(),
        });

        self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v3::wire::frame_kind::DatagramKind::Notify,
            payload: encoded,
        }).await.map_err(|e| ClientError::Send(format!("{e:?}")))
    }

    /// Send a bulk payload on a specific stream_id.
    ///
    /// Attempts memfd handoff (Linux, payload >= threshold, handoff enabled)
    /// before falling back to socket bulk. The handoff and socket paths
    /// share the same pending_bulk mechanism for result delivery.
    pub async fn send_bulk(
        &self,
        stream_id: u8,
        payload: &[u8],
        ack_timeout: Duration,
    ) -> Result<BulkDelivered, BulkError> {
        if !self.phase.lock().can_send() {
            return Err(BulkError::ConnectionLost);
        }

        // Atomic check-and-insert: prevent concurrent transfers on same stream_id
        {
            let pending = self.pending_bulk.lock();
            if pending.contains_key(&stream_id) {
                return Err(BulkError::StreamBusy);
            }
        }

        // ── Handoff fast path (Linux only) ────────────────────────────
        #[cfg(target_os = "linux")]
        if payload.len() as u64 >= self.handoff_threshold
            && self.handoff_enabled.load(Ordering::Relaxed)
        {
            match self.try_handoff(stream_id, payload, ack_timeout).await {
                Ok(delivered) => return Ok(delivered),
                Err(HandoffAttempt::Rejected) => {
                    // Fall through to socket bulk
                }
                Err(HandoffAttempt::Fatal(e)) => return Err(e),
            }
        }

        // ── Socket bulk path ──────────────────────────────────────────
        let (result_tx, result_rx) = oneshot::channel();
        self.pending_bulk.lock().insert(stream_id, BulkWaiter {
            tx: result_tx,
            started_at: Instant::now(),
        });

        let chunk_count = match self.bulk_sender.send(stream_id, payload, self.agreed_clearance).await {
            Ok(count) => count,
            Err(_) => {
                self.pending_bulk.lock().remove(&stream_id);
                return Err(BulkError::ConnectionLost);
            }
        };

        tracing::debug!(stream_id, chunk_count, timeout_secs = ack_timeout.as_secs(), "send_bulk: awaiting STREAM_ACK");
        match tokio::time::timeout(ack_timeout, result_rx).await {
            Ok(Ok(result)) => {
                tracing::debug!(stream_id, "send_bulk: ACK received");
                result
            }
            Ok(Err(_)) => {
                tracing::error!(stream_id, "send_bulk: pending_bulk oneshot closed — connection lost");
                Err(BulkError::ConnectionLost)
            }
            Err(_) => {
                tracing::error!(stream_id, chunk_count, timeout_secs = ack_timeout.as_secs(),
                    "send_bulk: ACK TIMEOUT — FIN/ACK cycle did not complete");
                self.pending_bulk.lock().remove(&stream_id);
                Err(BulkError::AckTimeout {
                    bytes_sent: payload.len() as u64,
                    chunks_sent: chunk_count as u64,
                })
            }
        }
    }

    /// Attempt memfd handoff for a bulk payload. Returns Ok on success,
    /// Err(Rejected) to fall back to socket bulk, Err(Fatal) for
    /// unrecoverable errors.
    #[cfg(target_os = "linux")]
    async fn try_handoff(
        &self,
        stream_id: u8,
        payload: &[u8],
        ack_timeout: Duration,
    ) -> Result<BulkDelivered, HandoffAttempt> {
        let content_hash = *blake3::hash(payload).as_bytes();
        let handoff_id = uuid::Uuid::now_v7();

        let offer = offer_codec::HandoffOfferPayload {
            handoff_id,
            fd_kind: 0x01,
            fd_sealed: true,
            stream_id,
            offer_timeout_ms: u32::try_from(ack_timeout.as_millis()).unwrap_or(u32::MAX),
            payload_size_bytes: payload.len() as u64,
            content_hash,
        };
        let offer_bytes = offer_codec::encode(&offer);

        // Register pending_bulk waiter for the handoff result
        let (result_tx, result_rx) = oneshot::channel();
        self.pending_bulk.lock().insert(stream_id, BulkWaiter {
            tx: result_tx,
            started_at: Instant::now(),
        });

        // Send HANDOFF_OFFER through control loop → Handoff lane → peer.
        // The control loop's outbound HANDOFF_OFFER handler:
        //   1. spawn_blocking → memfd_create + write payload + seal + BLAKE3 verify
        //   2. Send memfd fd via SCM_RIGHTS on the SOCK_SEQPACKET sidechannel
        //   3. Encode and send the offer frame to the peer
        //   4. Peer responds with HANDOFF_ACCEPT or HANDOFF_REJECT
        //   5. Handler resolves pending_bulk with BulkDelivered or BulkError
        if self.outbound_tx.send(OutboundFrame::Handoff {
            kind: crate::v3::wire::frame_kind::HandoffKind::Offer,
            payload: offer_bytes,
        }).await.is_err() {
            self.pending_bulk.lock().remove(&stream_id);
            return Err(HandoffAttempt::Fatal(BulkError::ConnectionLost));
        }

        // Await handoff result
        match tokio::time::timeout(ack_timeout, result_rx).await {
            Ok(Ok(Ok(delivered))) => Ok(delivered),
            Ok(Ok(Err(BulkError::PeerRejected { .. }))) => {
                // Peer rejected — clean up and signal fallback
                self.pending_bulk.lock().remove(&stream_id);
                Err(HandoffAttempt::Rejected)
            }
            Ok(Ok(Err(e))) => {
                self.pending_bulk.lock().remove(&stream_id);
                Err(HandoffAttempt::Fatal(e))
            }
            Ok(Err(_)) => {
                self.pending_bulk.lock().remove(&stream_id);
                Err(HandoffAttempt::Fatal(BulkError::ConnectionLost))
            }
            Err(_) => {
                // Timeout — clean up and fall back to socket bulk
                self.pending_bulk.lock().remove(&stream_id);
                Err(HandoffAttempt::Rejected)
            }
        }
    }

    /// Cancel an in-flight outbound bulk transfer.
    pub async fn cancel_bulk(&self, stream_id: u8) {
        use crate::v3::codec::stream::cancel as cancel_codec;
        use crate::v3::wire::frame_kind::StreamKind;

        let waiter = self.pending_bulk.lock().remove(&stream_id);
        if let Some(waiter) = waiter {
            let cancel = cancel_codec::encode(
                &cancel_codec::StreamCancelPayload {
                    transfer_id: uuid::Uuid::nil(),
                    bytes_through: 0,
                    chunks_through: 0,
                },
            );
            let _ = self.outbound_tx.send(OutboundFrame::Data {
                stream_id,
                kind: StreamKind::Cancel,
                chunk_index: 0,
                payload: cancel,
            }).await;
            let _ = waiter.tx.send(Err(BulkError::Cancelled));
        }
    }

    /// Initiate key rotation. The control loop calls `rotation.initiate()`
    /// to produce ROTATE_INIT, transitions to Rotating state, and sends the
    /// frame. The peer's `handle_init` responds with ROTATE_COMMIT. The
    /// local `handle_commit` derives new keys and resolves the completion
    /// oneshot. Both sides now have matching new keys.
    ///
    /// Returns Ok(()) after both sides have rotated. Returns Err on timeout,
    /// connection loss, or rotation failure (e.g., already rotating).
    pub async fn rotate_keys(&self, timeout: Duration) -> Result<(), SendError> {
        if !self.phase.lock().can_send() {
            return Err(SendError::ConnectionNotActive);
        }

        // Two oneshots: confirm (INIT sent) + completion (COMMIT processed).
        let (init_confirm_tx, init_confirm_rx) = oneshot::channel();
        let (rotation_complete_tx, rotation_complete_rx) = oneshot::channel();

        // Send via sequenced channel. The control loop:
        // 1. Stores rotation_complete_tx in ctx.pending_rotation_confirm
        // 2. drain_outbound detects empty-payload RotateInit, calls initiate()
        // 3. Encodes + sends the real ROTATE_INIT frame
        // 4. Resolves init_confirm_tx (INIT is on the wire)
        // 5. Server responds with ROTATE_COMMIT
        // 6. handle_commit calls receive_commit(), derives new keys,
        //    resolves rotation_complete_tx
        self.sequenced_tx.send(crate::v3::context::SequencedOutbound {
            frame: OutboundFrame::Channel {
                kind: crate::v3::wire::frame_kind::ChannelKind::RotateInit,
                payload: vec![],
            },
            confirm: init_confirm_tx,
            completion: Some(rotation_complete_tx),
        }).await.map_err(|_| SendError::ConnectionNotActive)?;

        // Await INIT sent — fail fast if control loop is dead.
        tokio::time::timeout(timeout, init_confirm_rx)
            .await
            .map_err(|_| SendError::AckTimeout)?
            .map_err(|_| SendError::ConnectionNotActive)?;

        // Await COMMIT processed — fail-closed, no polling, no probing.
        // handle_commit resolves this after receive_commit() + set_keys().
        tokio::time::timeout(timeout, rotation_complete_rx)
            .await
            .map_err(|_| SendError::AckTimeout)?
            .map_err(|_| SendError::ConnectionNotActive)?
            .map_err(|e| SendError::WriteFailed(format!("rotation failed: {e:?}")))
    }

    /// Send CHANNEL_GOODBYE for graceful shutdown.
    pub(crate) async fn send_goodbye(&self) -> Result<(), ClientError> {
        let goodbye = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
            reason_code: 0,
            drain_timeout_ms: 5000,
            final_session_seq: 0,
        });
        self.outbound_tx.send(OutboundFrame::Channel {
            kind: crate::v3::wire::frame_kind::ChannelKind::Goodbye,
            payload: goodbye,
        }).await.map_err(|e| ClientError::Send(format!("{e:?}")))
    }
}

/// Internal result type for handoff attempts.
#[cfg(target_os = "linux")]
enum HandoffAttempt {
    /// Peer rejected or timeout — fall back to socket bulk.
    Rejected,
    /// Unrecoverable error — propagate to caller.
    Fatal(BulkError),
}

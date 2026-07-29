//! Outbound operations — request/reply, bulk transfer, subscription.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::v4::bulk::send::BulkSender;
use crate::v4::codec::channel::goodbye as goodbye_codec;
use crate::v4::codec::datagram::request as request_codec;
use crate::v4::codec::datagram::notify as notify_codec;
use crate::v4::wire::outbound::{OutboundFrame, SequencedOutbound};
use crate::v4::wire::clearance::Clearance;

use super::types::{
    ClientError, SendDelivered, SendError, BulkDelivered, BulkError, ClientPhase,
    ReplyPayload, ReplySender, RequestReplyError,
};

pub(crate) struct AckWaiter {
    pub tx: oneshot::Sender<Result<(), SendError>>,
}

pub(crate) struct BulkWaiter {
    pub tx: oneshot::Sender<Result<BulkDelivered, BulkError>>,
    pub started_at: Instant,
}

pub(crate) struct SendState {
    pub outbound_tx: tokio::sync::mpsc::UnboundedSender<OutboundFrame>,
    pub sequenced_tx: tokio::sync::mpsc::Sender<SequencedOutbound>,
    pub bulk_sender: BulkSender,
    pub pending_acks: Arc<Mutex<HashMap<uuid::Uuid, AckWaiter>>>,
    pub pending_bulk: Arc<Mutex<HashMap<u8, BulkWaiter>>>,
    pub pending_replies: Arc<Mutex<HashMap<uuid::Uuid, ReplySender>>>,
    pub phase: Arc<Mutex<ClientPhase>>,
    pub agreed_clearance: Clearance,
}

impl SendState {
    pub async fn send_request(
        &self,
        payload: &[u8],
        ack_timeout: Duration,
    ) -> Result<SendDelivered, SendError> {
        let phase = *self.phase.lock();
        if !phase.can_send() {
            tracing::error!(?phase, "send_request: phase not sendable");
            return Err(SendError::ConnectionNotActive);
        }

        let message_id = uuid::Uuid::now_v7();
        tracing::debug!(
            %message_id,
            payload_len = payload.len(),
            timeout_ms = ack_timeout.as_millis() as u64,
            "send_request: entered"
        );

        let encoded = request_codec::encode(&request_codec::DatagramRequestPayload {
            message_id,
            reply_timeout_ms: u32::try_from(ack_timeout.as_millis()).unwrap_or(u32::MAX),
            sender_clearance: self.agreed_clearance,
            application_payload: payload.to_vec(),
            conditions: vec![],
        });

        let (ack_tx, ack_rx) = oneshot::channel();
        self.pending_acks.lock().insert(message_id, AckWaiter { tx: ack_tx });

        tracing::debug!(%message_id, "send_request: sending to outbound_tx");
        if let Err(e) = self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v4::wire::frame_kind::DatagramKind::Request,
            payload: encoded,
        }) {
            self.pending_acks.lock().remove(&message_id);
            tracing::error!(%message_id, error = %e, "send_request: outbound_tx send failed");
            return Err(SendError::WriteFailed(format!("{e:?}")));
        }
        tracing::debug!(%message_id, "send_request: sent, awaiting ACK");

        match tokio::time::timeout(ack_timeout, ack_rx).await {
            Ok(Ok(Ok(()))) => {
                tracing::debug!(%message_id, "send_request: ACK received");
                Ok(SendDelivered)
            }
            Ok(Ok(Err(send_err))) => {
                tracing::error!(%message_id, error = ?send_err, "send_request: peer error");
                Err(send_err)
            }
            Ok(Err(_)) => {
                tracing::error!(%message_id, "send_request: ack channel closed");
                Err(SendError::WriteFailed("channel closed".into()))
            }
            Err(_) => {
                self.pending_acks.lock().remove(&message_id);
                tracing::error!(%message_id, timeout_ms = ack_timeout.as_millis() as u64, "send_request: ACK timeout");
                Err(SendError::AckTimeout)
            }
        }
    }

    pub async fn request_reply(
        &self,
        payload: &[u8],
        reply_timeout: Duration,
    ) -> Result<ReplyPayload, RequestReplyError> {
        let phase = *self.phase.lock();
        if !phase.can_send() {
            tracing::error!(?phase, "request_reply: phase not sendable");
            return Err(RequestReplyError::ConnectionNotActive);
        }

        let message_id = uuid::Uuid::now_v7();
        tracing::debug!(
            %message_id,
            payload_len = payload.len(),
            timeout_ms = reply_timeout.as_millis() as u64,
            "request_reply: entered"
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

        tracing::debug!(%message_id, "request_reply: sending to outbound_tx");
        if let Err(e) = self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v4::wire::frame_kind::DatagramKind::Request,
            payload: encoded,
        }) {
            self.pending_replies.lock().remove(&message_id);
            tracing::error!(%message_id, error = %e, "request_reply: outbound channel failed");
            return Err(RequestReplyError::WriteFailed(format!("{e:?}")));
        }
        tracing::debug!(%message_id, "request_reply: sent, awaiting reply");

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

    pub async fn send_notify(&self, payload: &[u8]) -> Result<(), ClientError> {
        let message_id = uuid::Uuid::now_v7();
        tracing::debug!(
            %message_id,
            payload_len = payload.len(),
            "send_notify: entered"
        );
        let encoded = notify_codec::encode(&notify_codec::DatagramNotifyPayload {
            message_id,
            sender_clearance: self.agreed_clearance,
            application_payload: payload.to_vec(),
        });

        let result = self.outbound_tx.send(OutboundFrame::Datagram {
            kind: crate::v4::wire::frame_kind::DatagramKind::Notify,
            payload: encoded,
        });
        if result.is_err() {
            tracing::error!(%message_id, "send_notify: outbound_tx send failed");
        } else {
            tracing::debug!(%message_id, "send_notify: sent");
        }
        result.map_err(|e| ClientError::Send(format!("{e:?}")))
    }

    pub async fn send_bulk(
        &self,
        stream_id: u8,
        payload: &[u8],
        ack_timeout: Duration,
    ) -> Result<BulkDelivered, BulkError> {
        tracing::debug!(
            stream_id,
            payload_len = payload.len(),
            timeout_secs = ack_timeout.as_secs(),
            "send_bulk: entered"
        );

        let phase = *self.phase.lock();
        if !phase.can_send() {
            tracing::error!(stream_id, ?phase, "send_bulk: phase not sendable — returning ConnectionLost");
            return Err(BulkError::ConnectionLost);
        }
        tracing::debug!(stream_id, ?phase, "send_bulk: phase OK");

        {
            let pending = self.pending_bulk.lock();
            if pending.contains_key(&stream_id) {
                tracing::error!(stream_id, "send_bulk: stream_id already busy — returning StreamBusy");
                return Err(BulkError::StreamBusy);
            }
        }
        tracing::debug!(stream_id, "send_bulk: stream_id available, registering waiter");

        let (result_tx, result_rx) = oneshot::channel();
        self.pending_bulk.lock().insert(stream_id, BulkWaiter {
            tx: result_tx,
            started_at: Instant::now(),
        });

        tracing::debug!(stream_id, "send_bulk: calling bulk_sender.send()");
        let chunk_count = match self.bulk_sender.send(stream_id, payload, self.agreed_clearance).await {
            Ok(count) => {
                tracing::debug!(stream_id, chunk_count = count, "send_bulk: bulk_sender.send() returned OK");
                count
            }
            Err(e) => {
                tracing::error!(stream_id, error = ?e, "send_bulk: bulk_sender.send() FAILED — returning ConnectionLost");
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

    pub async fn cancel_bulk(&self, stream_id: u8) {
        use crate::v4::codec::stream::cancel as cancel_codec;
        use crate::v4::wire::frame_kind::StreamKind;

        tracing::debug!(stream_id, "cancel_bulk: entered");
        let waiter = self.pending_bulk.lock().remove(&stream_id);
        if let Some(waiter) = waiter {
            tracing::debug!(stream_id, "cancel_bulk: waiter found, sending STREAM_CANCEL");
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
            });
            let _ = waiter.tx.send(Err(BulkError::Cancelled));
            tracing::debug!(stream_id, "cancel_bulk: cancel sent, waiter resolved");
        } else {
            tracing::debug!(stream_id, "cancel_bulk: no waiter — nothing to cancel");
        }
    }

    pub async fn rotate_keys(&self, timeout: Duration) -> Result<(), SendError> {
        let phase = *self.phase.lock();
        if !phase.can_send() {
            tracing::error!(?phase, "rotate_keys: phase not sendable");
            return Err(SendError::ConnectionNotActive);
        }

        let (init_confirm_tx, init_confirm_rx) = oneshot::channel();
        let (rotation_complete_tx, rotation_complete_rx) = oneshot::channel();

        tracing::debug!(timeout_ms = timeout.as_millis() as u64, "rotate_keys: sending sequenced ROTATE_INIT");
        self.sequenced_tx.send(SequencedOutbound {
            frame: OutboundFrame::Channel {
                kind: crate::v4::wire::frame_kind::ChannelKind::RotateInit,
                payload: vec![],
            },
            confirm: init_confirm_tx,
            completion: Some(rotation_complete_tx),
        }).await.map_err(|_| {
            tracing::error!("rotate_keys: sequenced_tx send failed");
            SendError::ConnectionNotActive
        })?;
        tracing::debug!("rotate_keys: sequenced send OK, awaiting INIT confirmation");

        tokio::time::timeout(timeout, init_confirm_rx)
            .await
            .map_err(|_| {
                tracing::error!("rotate_keys: INIT confirmation TIMEOUT");
                SendError::AckTimeout
            })?
            .map_err(|_| {
                tracing::error!("rotate_keys: INIT confirmation channel CLOSED");
                SendError::ConnectionNotActive
            })?;
        tracing::debug!("rotate_keys: INIT confirmed, awaiting COMMIT completion");

        tokio::time::timeout(timeout, rotation_complete_rx)
            .await
            .map_err(|_| {
                tracing::error!("rotate_keys: COMMIT completion TIMEOUT");
                SendError::AckTimeout
            })?
            .map_err(|_| {
                tracing::error!("rotate_keys: COMMIT completion channel CLOSED");
                SendError::ConnectionNotActive
            })?
            .map_err(|e| {
                tracing::error!(error = ?e, "rotate_keys: rotation FAILED");
                SendError::WriteFailed(format!("rotation failed: {e:?}"))
            })?;
        tracing::debug!("rotate_keys: rotation complete");
        Ok(())
    }

    pub(crate) async fn send_goodbye(&self) -> Result<(), ClientError> {
        tracing::debug!("send_goodbye: entered");
        let goodbye = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
            reason_code: 0,
            drain_timeout_ms: 5000,
            final_session_seq: 0,
        });
        let result = self.outbound_tx.send(OutboundFrame::Channel {
            kind: crate::v4::wire::frame_kind::ChannelKind::Goodbye,
            payload: goodbye,
        });
        if result.is_err() {
            tracing::error!("send_goodbye: outbound_tx send failed");
        } else {
            tracing::debug!("send_goodbye: GOODBYE sent");
        }
        result.map_err(|e| ClientError::Send(format!("{e:?}")))
    }
}

//! DaemonRouter — bridges transport-ipc's FrameRouter to daemon dispatch.
//!
//! One instance per accepted connection. Created by the factory closure
//! in `IpcServer::bind()`. Dropped when the connection closes.

use std::sync::Arc;

use rekindle_transport_ipc::v3::codec::datagram::reply as reply_codec;
use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::router::{ConnectionInfo, ConnectionPhase, FrameRouter};
use rekindle_transport_ipc::v3::server::ConnectionHandle;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_kind::DatagramKind;
use rekindle_types::daemon::{ChatRequest, DaemonRequest, DaemonResponse, LifecycleRequest};

use crate::daemon::dispatch::{dispatch, DaemonContext};
use crate::subscriptions::SubscriptionRegistry;

/// Bridges transport-ipc's frame callbacks to the daemon's dispatch system.
pub struct DaemonRouter {
    handle: ConnectionHandle,
    ctx: Arc<DaemonContext>,
    subs: Arc<SubscriptionRegistry>,
}

impl DaemonRouter {
    pub fn new(
        handle: ConnectionHandle,
        ctx: Arc<DaemonContext>,
        subs: Arc<SubscriptionRegistry>,
    ) -> Self {
        Self { handle, ctx, subs }
    }

    fn send_reply(&self, correlation_id: uuid::Uuid, response: &DaemonResponse) {
        let app_payload = match response.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "DaemonResponse serialize failed");
                return;
            }
        };

        let reply = reply_codec::encode(&reply_codec::DatagramReplyPayload {
            message_id: uuid::Uuid::now_v7(),
            correlation_id,
            status_phase: 0x03,
            application_payload: app_payload,
            status_reason: String::new(),
            status_message: String::new(),
            conditions: vec![],
        });

        let _ = self.handle.outbound_tx.try_send(OutboundFrame::Datagram {
            kind: DatagramKind::Reply,
            payload: reply,
        });
    }
}

impl FrameRouter for DaemonRouter {
    fn on_request(
        &self,
        info: &ConnectionInfo,
        message_id: uuid::Uuid,
        _clearance: Clearance,
        payload: &[u8],
    ) {
        let request: DaemonRequest = match DaemonRequest::from_bytes(payload) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    conn_id = info.conn_id,
                    error = %e,
                    "DaemonRequest decode failed"
                );
                self.send_reply(
                    message_id,
                    &DaemonResponse::error(400, format!("request decode: {e}")),
                );
                return;
            }
        };

        tracing::debug!(conn_id = info.conn_id, request = ?request, "dispatching");

        // Subscribe/Unsubscribe: server-side, synchronous
        match &request {
            DaemonRequest::Lifecycle(LifecycleRequest::Subscribe { filters }) => {
                tracing::info!(
                    conn_id = info.conn_id,
                    filter_count = filters.len(),
                    "routing: Subscribe registered"
                );
                self.subs.subscribe(
                    info.conn_id,
                    self.handle.clone(),
                    filters.clone(),
                );
                self.send_reply(
                    message_id,
                    &DaemonResponse::ok(&serde_json::json!({"subscribed": true})),
                );
                return;
            }
            DaemonRequest::Lifecycle(LifecycleRequest::Unsubscribe { filters }) => {
                self.subs.unsubscribe(info.conn_id, filters);
                self.send_reply(
                    message_id,
                    &DaemonResponse::ok(&serde_json::json!({"unsubscribed": true})),
                );
                return;
            }
            _ => {}
        }

        // Idempotency check
        let client_msg_id = match &request {
            DaemonRequest::Chat(ChatRequest::ChannelSend {
                client_msg_id: Some(id),
                ..
            }) => {
                if let Some(cached) = self.ctx.idempotency_cache.check(id) {
                    self.send_reply(message_id, &cached);
                    return;
                }
                Some(id.clone())
            }
            _ => None,
        };

        // Async dispatch
        let ctx = Arc::clone(&self.ctx);
        let handle = self.handle.clone();
        let cache_key = client_msg_id;

        tokio::spawn(async move {
            let response = dispatch(&ctx, request, None).await;

            if let Some(key) = cache_key {
                ctx.idempotency_cache.store(key, response.clone());
            }

            let app_payload = match response.to_bytes() {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!(error = %e, "response serialize failed");
                    return;
                }
            };

            let reply = reply_codec::encode(&reply_codec::DatagramReplyPayload {
                message_id: uuid::Uuid::now_v7(),
                correlation_id: message_id,
                status_phase: 0x03,
                application_payload: app_payload,
                status_reason: String::new(),
                status_message: String::new(),
                conditions: vec![],
            });

            let _ = handle.outbound_tx.try_send(OutboundFrame::Datagram {
                kind: DatagramKind::Reply,
                payload: reply,
            });
        });
    }

    fn on_bulk_complete(
        &self,
        info: &ConnectionInfo,
        stream_id: u8,
        transfer_id: uuid::Uuid,
        total_bytes: u64,
        _chunks: u32,
    ) {
        tracing::info!(
            conn_id = info.conn_id,
            stream_id,
            %transfer_id,
            total_bytes,
            "bulk transfer complete"
        );
    }

    fn on_bulk_failed(
        &self,
        info: &ConnectionInfo,
        stream_id: u8,
        transfer_id: uuid::Uuid,
        reason: &str,
    ) {
        tracing::warn!(
            conn_id = info.conn_id,
            stream_id,
            %transfer_id,
            reason,
            "bulk transfer failed"
        );
    }

    fn on_connection_state_change(
        &self,
        info: &ConnectionInfo,
        old: ConnectionPhase,
        new: ConnectionPhase,
    ) {
        tracing::info!(
            conn_id = info.conn_id,
            old_phase = %old,
            new_phase = %new,
            "connection phase transition"
        );
        if matches!(new, ConnectionPhase::Closed | ConnectionPhase::Dead) {
            self.subs.remove_connection(info.conn_id);
        }
    }

    fn on_notify(&self, _: &ConnectionInfo, _: uuid::Uuid, _: Clearance, _: &[u8]) {}
    fn on_publish(&self, _: &ConnectionInfo, _: uuid::Uuid, _: &[u8; 32], _: u32, _: &[u8]) {}
    fn on_reply(&self, _: &ConnectionInfo, _: uuid::Uuid, _: uuid::Uuid, _: u32, _: &[u8]) {}
    fn on_reject(&self, _: &ConnectionInfo, _: uuid::Uuid, _: u32, _: &str) {}
    fn on_ack(&self, _: &ConnectionInfo, _: &[uuid::Uuid]) {}
    fn route_frame(&self, _: &ConnectionInfo, _: u8, _: u8, _: &[u8]) {}
}

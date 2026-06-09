//! Inbound operations — application frames, bulk chunks, reassembled payloads.
//!
//! All recv methods take `&self` (not `&mut self`) so the application can
//! send and receive concurrently on the same IpcClient. Internal mutability
//! via tokio::sync::Mutex on the receive channels. The Mutex is uncontended
//! in the common case (one recv task per client) and serializes correctly
//! when multiple tasks recv simultaneously.

use std::collections::{HashSet, VecDeque};

use tokio::sync::{mpsc, Mutex};

use super::types::{BulkChunk, InboundFrame};

/// Inbound receiver state. All methods take `&self` for concurrent send+recv.
pub struct InboundReceiver {
    inbound_rx: Mutex<mpsc::Receiver<InboundFrame>>,
    bulk_chunk_rx: Mutex<mpsc::Receiver<BulkChunk>>,
    cancelled_recv_streams: std::sync::Arc<parking_lot::Mutex<HashSet<u8>>>,
    buffered_chunks: Mutex<VecDeque<BulkChunk>>,
}

impl InboundReceiver {
    pub fn new(
        inbound_rx: mpsc::Receiver<InboundFrame>,
        bulk_chunk_rx: mpsc::Receiver<BulkChunk>,
        cancelled_recv_streams: std::sync::Arc<parking_lot::Mutex<HashSet<u8>>>,
    ) -> Self {
        Self {
            inbound_rx: Mutex::new(inbound_rx),
            bulk_chunk_rx: Mutex::new(bulk_chunk_rx),
            cancelled_recv_streams,
            buffered_chunks: Mutex::new(VecDeque::new()),
        }
    }

    /// Receive next inbound application frame. None on disconnect.
    pub async fn recv(&self) -> Option<InboundFrame> {
        self.inbound_rx.lock().await.recv().await
    }

    /// Receive next bulk data chunk (streaming API).
    /// Chunks arrive in decryption-completion order, NOT chunk_index order.
    /// The chunk_index field tells the application where to position the data.
    pub async fn recv_bulk_chunk(&self) -> Option<BulkChunk> {
        let mut buffered = self.buffered_chunks.lock().await;
        self.drain_cancelled(&mut buffered);

        if let Some(chunk) = buffered.pop_front() {
            return Some(chunk);
        }
        drop(buffered);

        let mut rx = self.bulk_chunk_rx.lock().await;
        loop {
            let chunk = rx.recv().await?;
            if self.cancelled_recv_streams.lock().contains(&chunk.stream_id) {
                continue;
            }
            return Some(chunk);
        }
    }

    /// Receive a complete bulk payload (buffered reassembly).
    /// Accumulates chunks into a contiguous Vec<u8>.
    /// Handles multi-stream interleaving via internal VecDeque buffer.
    pub async fn recv_bulk(&self) -> Option<(u8, Vec<u8>)> {
        let mut payload = Vec::new();
        let mut target_sid: Option<u8> = None;

        loop {
            if let Some(sid) = target_sid {
                if self.cancelled_recv_streams.lock().contains(&sid) {
                    payload.clear();
                    target_sid = None;
                    continue;
                }
            }

            let chunk = if let Some(sid) = target_sid {
                let mut buffered = self.buffered_chunks.lock().await;
                if let Some(idx) = buffered.iter().position(|c| c.stream_id == sid) {
                    buffered.remove(idx).unwrap()
                } else {
                    drop(buffered);
                    let mut rx = self.bulk_chunk_rx.lock().await;
                    rx.recv().await?
                }
            } else {
                let mut buffered = self.buffered_chunks.lock().await;
                self.drain_cancelled(&mut buffered);
                if let Some(idx) = buffered.iter().position(|c| {
                    !self.cancelled_recv_streams.lock().contains(&c.stream_id)
                }) {
                    buffered.remove(idx).unwrap()
                } else {
                    drop(buffered);
                    let mut rx = self.bulk_chunk_rx.lock().await;
                    loop {
                        let chunk = rx.recv().await?;
                        if !self.cancelled_recv_streams.lock().contains(&chunk.stream_id) {
                            break chunk;
                        }
                    }
                }
            };

            let sid = chunk.stream_id;
            if self.cancelled_recv_streams.lock().contains(&sid) {
                continue;
            }

            let first_sid = *target_sid.get_or_insert(sid);

            if sid != first_sid {
                self.buffered_chunks.lock().await.push_back(chunk);
                continue;
            }

            if chunk.is_last {
                return Some((sid, payload));
            }
            payload.extend_from_slice(chunk.payload());
        }
    }

    /// Cancel an in-flight inbound bulk receive on a specific stream.
    pub fn cancel_recv_bulk(&self, stream_id: u8) {
        self.cancelled_recv_streams.lock().insert(stream_id);
    }

    fn drain_cancelled(&self, buffered: &mut VecDeque<BulkChunk>) {
        let cancelled = self.cancelled_recv_streams.lock();
        if cancelled.is_empty() {
            return;
        }
        buffered.retain(|c| !cancelled.contains(&c.stream_id));
    }
}

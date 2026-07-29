//! Per-session stream ID registry — tracks up to 256 concurrent streams per direction.

use super::state::{StreamEvent, StreamState, StreamTransitionError};
use std::collections::HashMap;

/// Direction qualifier for stream registry keys. Prevents collision when
/// the same stream_id is used for both inbound and outbound transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Inbound,
    Outbound,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inbound => write!(f, "inbound"),
            Self::Outbound => write!(f, "outbound"),
        }
    }
}

#[derive(Debug)]
pub enum RegistryError {
    AlreadyOpen(u8),
    NotFound(u8),
    TransitionFailed(u8, StreamTransitionError),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyOpen(id) => write!(f, "StreamId {id} AlreadyOpen"),
            Self::NotFound(id) => write!(f, "StreamId {id} not found"),
            Self::TransitionFailed(id, err) => write!(f, "StreamId {id}: {err}"),
        }
    }
}

#[derive(Default)]
pub struct StreamRegistry {
    streams: HashMap<(u8, Direction), StreamState>,
}

impl StreamRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, id: u8, direction: Direction) -> Result<(), RegistryError> {
        let key = (id, direction);
        if let Some(state) = self.streams.get(&key) {
            match (*state, direction) {
                // Closed/Failed: normal reopen after completed lifecycle.
                (StreamState::Closed | StreamState::Failed, _) => {}
                // Outbound re-open while previous lifecycle is still in-flight
                // (ACK not yet processed). The new STREAM_OPEN carries a fresh
                // transfer_id — force-close the stale entry and proceed.
                (_, Direction::Outbound) => {
                    tracing::debug!(
                        stream_id = id,
                        %direction,
                        old_state = ?state,
                        "StreamRegistry::open — force-closing stale outbound stream for reopen"
                    );
                }
                // Inbound: reject genuine collisions — the peer must not
                // reuse a stream_id that is still actively open inbound.
                (_, Direction::Inbound) => {
                    tracing::error!(
                        stream_id = id,
                        %direction,
                        current_state = ?state,
                        all_streams = ?self.streams,
                        "StreamRegistry::open REJECTED — stream already open"
                    );
                    return Err(RegistryError::AlreadyOpen(id));
                }
            }
        }
        tracing::debug!(stream_id = id, %direction, "StreamRegistry::open — stream opened");
        self.streams.insert(key, StreamState::Open);
        Ok(())
    }

    pub fn close(&mut self, id: u8, direction: Direction) -> Result<(), RegistryError> {
        let key = (id, direction);
        match self.streams.get_mut(&key) {
            Some(state) => {
                tracing::debug!(stream_id = id, %direction, old_state = ?state, "StreamRegistry::close");
                *state = StreamState::Closed;
                Ok(())
            }
            None => {
                tracing::error!(stream_id = id, %direction, "StreamRegistry::close — stream not found");
                Err(RegistryError::NotFound(id))
            }
        }
    }

    pub fn reset(&mut self, id: u8, direction: Direction) -> Result<(), RegistryError> {
        let key = (id, direction);
        match self.streams.get_mut(&key) {
            Some(state) => {
                tracing::debug!(stream_id = id, %direction, old_state = ?state, "StreamRegistry::reset");
                *state = StreamState::Closed;
                Ok(())
            }
            None => {
                tracing::error!(stream_id = id, %direction, "StreamRegistry::reset — stream not found");
                Err(RegistryError::NotFound(id))
            }
        }
    }

    pub fn transition(&mut self, id: u8, direction: Direction, event: StreamEvent) -> Result<(), RegistryError> {
        let key = (id, direction);
        match self.streams.get_mut(&key) {
            Some(state) => {
                tracing::debug!(stream_id = id, %direction, old_state = ?state, event = ?event, "StreamRegistry::transition");
                state.apply(event)
                    .map_err(|e| RegistryError::TransitionFailed(id, e))
            }
            None => {
                tracing::error!(stream_id = id, %direction, event = ?event, "StreamRegistry::transition — stream not found");
                Err(RegistryError::NotFound(id))
            }
        }
    }

    pub fn state(&self, id: u8, direction: Direction) -> Option<StreamState> {
        self.streams.get(&(id, direction)).copied()
    }

    pub fn active_count(&self) -> usize {
        self.streams.values().filter(|s| {
            !matches!(s, StreamState::Closed | StreamState::Failed | StreamState::Idle)
        }).count()
    }
}

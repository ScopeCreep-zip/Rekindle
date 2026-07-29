use std::collections::HashMap;

use crate::v4::codec::channel::subscribe as codec;
use crate::v4::handlers::HandlerError;
use crate::v4::wire::outbound::{OutboundFrame, OutboundChannelKind};

// ── Handler ─────────────────────────────────────────────────────

pub fn handle(
    subscriptions: &mut SubscriptionRegistry,
    outbound: &mut Vec<OutboundFrame>,
    payload: &[u8],
) -> Result<(), HandlerError> {
    if subscriptions.is_full() {
        return Err(HandlerError::SubscriptionQuotaExhausted);
    }

    let sub = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let topic_hashes: Vec<[u8; 32]> = sub.topics.iter().map(|t| t.topic_hash).collect();
    subscriptions.register(sub.subscription_id, &topic_hashes, &sub.conditions);

    let ack = codec::SubscribeAckPayload {
        subscription_id: sub.subscription_id,
        accepted_hashes: topic_hashes,
    };
    outbound.push(OutboundFrame::Channel {
        kind: OutboundChannelKind::SubscribeAck,
        payload: codec::encode_ack(&ack),
    });

    Ok(())
}

// ── Subscription types ──────────────────────────────────────────

/// A pub-sub subscription with topic filters and optional conditions.
pub struct Subscription {
    pub topics: Vec<[u8; 32]>,
    pub conditions: Vec<u8>,
}

/// Tracks active subscriptions with a reverse topic index for O(1) lookup.
/// Owned by the Control lane task.
pub struct SubscriptionRegistry {
    subscriptions: HashMap<uuid::Uuid, Subscription>,
    topic_index: HashMap<[u8; 32], Vec<uuid::Uuid>>,
    max_subscriptions: usize,
}

impl SubscriptionRegistry {
    pub fn new(max_subscriptions: usize) -> Self {
        Self {
            subscriptions: HashMap::new(),
            topic_index: HashMap::new(),
            max_subscriptions,
        }
    }

    pub fn register(
        &mut self,
        sub_id: uuid::Uuid,
        topics: &[[u8; 32]],
        conditions: &[u8],
    ) {
        let topic_vec = topics.to_vec();
        for topic in &topic_vec {
            self.topic_index.entry(*topic).or_default().push(sub_id);
        }
        self.subscriptions.insert(sub_id, Subscription {
            topics: topic_vec,
            conditions: conditions.to_vec(),
        });
    }

    pub fn remove(&mut self, sub_id: &uuid::Uuid) -> bool {
        if let Some(sub) = self.subscriptions.remove(sub_id) {
            for topic in &sub.topics {
                if let Some(subs) = self.topic_index.get_mut(topic) {
                    subs.retain(|id| id != sub_id);
                    if subs.is_empty() {
                        self.topic_index.remove(topic);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    pub fn count(&self) -> usize {
        self.subscriptions.len()
    }

    pub fn is_full(&self) -> bool {
        self.subscriptions.len() >= self.max_subscriptions
    }

    pub fn has_topic(&self, topic: &[u8; 32]) -> bool {
        self.topic_index.contains_key(topic)
    }

    pub fn subscriptions_for_topic(&self, topic: &[u8; 32]) -> Vec<uuid::Uuid> {
        self.topic_index.get(topic).cloned().unwrap_or_default()
    }

    pub fn conditions(&self, sub_id: &uuid::Uuid) -> Option<&[u8]> {
        self.subscriptions.get(sub_id).map(|s| s.conditions.as_slice())
    }
}

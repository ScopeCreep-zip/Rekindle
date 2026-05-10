//! Watch registry: maps record_key → WatchKind for inbound event routing.

use std::collections::HashMap;

use parking_lot::RwLock;
use rekindle_transport::WatchToken;

#[derive(Debug, Clone)]
pub enum WatchKind {
    DmLog { peer_key: String },
    ChannelLog { community: String, channel_id: String, member: String },
    FriendInbox,
    GovernanceManifest { community: String },
    MemberRegistry { community: String },
    JoinInbox { community: String },
}

pub struct WatchRegistry {
    by_key: RwLock<HashMap<String, WatchKind>>,
    tokens: RwLock<HashMap<String, WatchToken>>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        Self {
            by_key: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
        }
    }

    pub fn register(&self, record_key: &str, kind: WatchKind, token: WatchToken) {
        self.by_key.write().insert(record_key.to_string(), kind);
        self.tokens.write().insert(record_key.to_string(), token);
    }

    pub fn unregister(&self, record_key: &str) -> Option<WatchToken> {
        self.by_key.write().remove(record_key);
        self.tokens.write().remove(record_key)
    }

    pub fn lookup(&self, record_key: &str) -> Option<WatchKind> {
        self.by_key.read().get(record_key).cloned()
    }

    pub fn all_tokens(&self) -> Vec<(String, WatchToken)> {
        self.tokens.read().iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    pub fn count(&self) -> usize {
        self.by_key.read().len()
    }
}

impl Default for WatchRegistry {
    fn default() -> Self {
        Self::new()
    }
}

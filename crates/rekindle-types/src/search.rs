//! Architecture §23 — local FTS5 search query/result types.
//!
//! Mirrors the `MessageSearch` / `SearchFilters` shape declared in
//! `.claude/docs/rekindle-communities-architecture.md` §23.2 (lines
//! 2679-2696). Pure data; the orchestration that translates these into
//! SQLite FTS5 queries lives in `src-tauri/src/services/search/`.

use serde::{Deserialize, Serialize};

/// `has` filter — restricts to messages that include a given attachment
/// kind (architecture §23.2 line 2686).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HasFilter {
    Link,
    File,
    Image,
    Video,
    Embed,
    Poll,
    VoiceMessage,
}

/// Result ordering (architecture §23.2 line 2692).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    #[default]
    Relevance,
    Newest,
    Oldest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Architecture §32 Phase 7 W23 line 4111 — community-scoped search.
    /// `None` = global (all communities); `Some(community_id)` restricts
    /// matches to messages in that community. Combine with `in_channel`
    /// for channel-scoped search inside a community.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_community: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_thread: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub has: Vec<HasFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_pinned: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSearch {
    pub query: String,
    #[serde(default)]
    pub filters: SearchFilters,
    #[serde(default)]
    pub sort: SearchSort,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    25
}

pub const MAX_SEARCH_LIMIT: u32 = 100;

/// Where the hit was found — `messages` covers channel + DM bridge,
/// `thread_messages` covers thread replies, `dm_messages` covers the
/// per-DM ratchet log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    Channel,
    Thread,
    Dm,
}

/// One result row. Architecture §23.2 line 2698 prescribes ±1 message
/// context — surfaced as `before_body` / `after_body`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub scope: SearchScope,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub rank: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_body: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub total_returned: u32,
    pub query_ms: u32,
}

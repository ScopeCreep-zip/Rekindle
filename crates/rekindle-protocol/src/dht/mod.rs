pub mod account;
pub mod community;
pub mod conversation;
pub mod friends;
pub mod log;
pub mod mailbox;
pub mod presence;
pub mod profile;
pub mod schema;
pub mod short_array;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use veilid_core::RoutingContext;

/// Parse a DHT record key string into a Veilid `RecordKey`.
/// Default bounded-retry budget for opening an existing DHT record before
/// concluding it is genuinely gone and recreating it. ≈45 s — long enough for
/// a freshly-attached node's routing table to mature (the architecture's
/// cold-start model: §4.8 "opened once on startup"; §2675 recreate is for
/// genuine >1 h expiry). Mirrors `allocate_route_with_retry`'s 15 attempts.
pub const DEFAULT_DHT_OPEN_ATTEMPTS: u32 = 15;
pub const DEFAULT_DHT_OPEN_DELAY: std::time::Duration = std::time::Duration::from_secs(3);

mod manager;
mod retry;
mod routes;

pub(crate) use retry::classify_dht_open_error;
pub use retry::{parse_record_key, retry_on_unreachable};

/// Result of an open-or-create DHT record operation.
pub struct OpenOrCreateResult {
    /// The DHT record key string.
    pub key: String,
    /// Owner keypair — the stored one on reuse, or freshly generated on create.
    pub keypair: Option<veilid_core::KeyPair>,
    /// `true` if a new record was created (keypair must be persisted).
    pub is_new: bool,
}

/// Manages DHT record operations (profiles, friend lists, communities).
///
/// Wraps a Veilid `RoutingContext` to perform record CRUD, watch, and get/set
/// operations on the distributed hash table.
pub struct DHTManager {
    /// Veilid routing context used for all DHT operations.
    routing_context: RoutingContext,
    /// Our own profile record key.
    pub profile_key: Option<String>,
    /// Our friend list record key.
    pub friend_list_key: Option<String>,
    /// Cached route blobs for known peers (`pubkey_hex` -> `route_blob`).
    pub route_cache: HashMap<String, Vec<u8>>,
    /// All record keys opened/created in this session, for bulk close on shutdown.
    pub open_records: HashSet<String>,
    /// Cache of imported route blobs → `(RouteId, import_time)` to prevent resource leaks.
    /// Without this, each call to `import_remote_private_route` leaks a `RouteId`.
    /// Entries older than [`IMPORTED_ROUTE_TTL_SECS`] are evicted and re-imported.
    pub imported_routes: HashMap<Vec<u8>, (veilid_core::RouteId, Instant)>,
    /// Reverse map from `RouteId` → pubkey hex for selective invalidation.
    pub route_id_to_pubkey: HashMap<veilid_core::RouteId, String>,
    /// Default writer keypair for `set_value` operations.
    ///
    /// When set, `set_value` will explicitly pass this keypair in
    /// `SetDHTValueOptions` instead of relying on the record's default writer
    /// (which can be lost if another code path re-opens the record read-only).
    default_writer: Option<veilid_core::KeyPair>,
}

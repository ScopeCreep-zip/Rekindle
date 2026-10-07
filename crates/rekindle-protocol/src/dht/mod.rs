pub mod account;
pub mod community;
pub mod conversation;
pub mod friends;
pub mod log;
pub mod mailbox;
pub mod pool;
pub mod profile;
pub mod route_imports;
pub mod schema;
pub mod short_array;

use crate::error::ProtocolError;

/// Parse a DHT record key string into a Veilid `RecordKey`.
///
/// # Errors
/// The string is not a record key.
pub fn parse_record_key(key: &str) -> Result<veilid_core::RecordKey, ProtocolError> {
    key.parse()
        .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))
}

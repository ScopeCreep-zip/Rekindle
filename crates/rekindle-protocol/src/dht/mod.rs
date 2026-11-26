pub mod account;
pub mod backend;
pub mod community;
pub mod friends;
pub mod mailbox;
pub mod pool;
pub mod prekeys;
pub mod profile;
pub mod route_imports;

use crate::error::ProtocolError;

/// Parse a DHT record key string into a Veilid `RecordKey`.
///
/// # Errors
/// The string is not a record key.
pub fn parse_record_key(key: &str) -> Result<veilid_core::RecordKey, ProtocolError> {
    key.parse()
        .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))
}

#[cfg(test)]
mod contested_tests;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

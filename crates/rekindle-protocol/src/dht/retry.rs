//! Transient-vs-hard DHT error classification and the bounded retry
//! entry point (built on `rekindle_utils::retry`).

use crate::error::ProtocolError;

/// Map a veilid DHT-open error to a `ProtocolError`, separating the
/// TRANSIENT "couldn't reach any node holding this record yet" case from hard
/// errors. On a freshly-attached node with a sparse routing table the outbound
/// fanout exhausts and the open returns `KeyNotFound` (or `TryAgain` /
/// `Timeout` / `NoConnection`) even though the record still exists — see
/// veilid-core `storage_manager/open_record.rs`. `TransactionNotFound` (new
/// in 0.5.4's transaction-aware watch/inspect) is likewise transient: the
/// referenced DHT transaction expired or lost to a concurrent record open,
/// and re-running the operation opens a fresh one. Those are mapped to
/// `DhtRecordUnreachable` so callers retry instead of recreating; every other
/// variant (e.g. `Generic` "not writable", `InvalidArgument`) is a hard
/// `DhtError` where retrying/recreating is pointless or wrong.
pub(crate) fn classify_dht_open_error(
    context: &str,
    e: &veilid_core::VeilidAPIError,
) -> ProtocolError {
    use veilid_core::VeilidAPIError as E;
    match e {
        E::KeyNotFound { .. }
        | E::TryAgain { .. }
        | E::Timeout
        | E::NoConnection { .. }
        | E::TransactionNotFound { .. } => {
            ProtocolError::DhtRecordUnreachable(format!("{context}: {e}"))
        }
        _ => ProtocolError::DhtError(format!("{context}: {e}")),
    }
}

/// Retry an async DHT-open operation while it fails with the TRANSIENT
/// [`ProtocolError::DhtRecordUnreachable`] (a sparse routing table on a
/// freshly-attached node), up to `attempts` times with `delay` between tries.
/// Success and HARD errors return immediately; after the budget is exhausted
/// the last unreachable error is returned so the caller can treat the record
/// as genuinely gone (and recreate). `delay = Duration::ZERO` disables
/// sleeping (tests). Thin wrapper over [`rekindle_utils::retry`] — the one
/// retry loop shared across the workspace.
pub async fn retry_on_unreachable<T, F, Fut>(
    attempts: u32,
    delay: std::time::Duration,
    op: F,
) -> Result<T, ProtocolError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, ProtocolError>>,
{
    rekindle_utils::retry::retry_with_backoff(
        rekindle_utils::retry::RetryPolicy::fixed(attempts, delay),
        "dht-open",
        |e| matches!(e, ProtocolError::DhtRecordUnreachable(_)),
        op,
    )
    .await
}

/// Parse a DHT record key string into a Veilid `RecordKey`.
pub fn parse_record_key(key: &str) -> Result<veilid_core::RecordKey, ProtocolError> {
    key.parse()
        .map_err(|e| ProtocolError::DhtError(format!("invalid record key '{key}': {e}")))
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

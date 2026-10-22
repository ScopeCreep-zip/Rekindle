//! Local full-text search over the stored messages (architecture §23,
//! plan E3.1 E3b), on both hosts: `rekindle_db::repo::search` over the
//! FTS5 tables, timed on the runtime's monotonic clock.

use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::search::{MessageSearch, SearchResult};

use super::HostGovernance;

impl HostGovernance {
    /// The unlocked identity's hits for `request`.
    ///
    /// # Errors
    /// Locked, or the query failed.
    pub(crate) async fn search_messages(
        &self,
        request: MessageSearch,
    ) -> Result<SearchResult, GovernanceRuntimeError> {
        let (owner, db) = self.owner_db()?;
        let started = tokio::time::Instant::now();
        let hits = db
            .call(move |conn| rekindle_db::repo::search::search(conn, &owner, &request))
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("search failed: {e}")))?;
        Ok(SearchResult {
            total_returned: u32::try_from(hits.len()).unwrap_or(u32::MAX),
            query_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            hits,
        })
    }
}

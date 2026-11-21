//! Our own sealed op pages, re-learned at unlock (plan E3.2 S7 record
//! warming, design step 3). The warm set persists nothing: at each login
//! stage, after the rebuild has applied the latest endorsed checkpoint,
//! our own op log is walked down from our local copy of its head
//! (`M`/`GOV_HEAD`) to that checkpoint's cut for our pseudonym, and each
//! page above it is held again (`SealedPages::own_page`). Every read is of
//! a local copy (`rekindle_records::chain::local_pages_after`), never the
//! network, and the walk is bounded by the cut: the pages it yields are
//! exactly the own pages the warm set holds beyond it.

use rekindle_records::chain::local_pages_after;
use rekindle_types::dht_layout::member_record::GOV_HEAD;

use super::HostGovernance;

impl HostGovernance {
    /// Hold again our own op pages above the held checkpoint's cut in
    /// `community_id`, from our local copies of our op log.
    pub(super) async fn rewarm_own_op_pages(&self, community_id: &str) {
        let own = self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .and_then(|c| {
                let me = super::watch_ring::my_pseudonym(c)?;
                let state = c.governance_state.as_ref()?;
                let record = state.members.get(&me)?.member_record.clone();
                Some((record, c.sealed_pages.own_floor()))
            });
        let Some((record, floor)) = own else {
            return;
        };
        let pool = match self.transport().map(|t| t.require_records()) {
            Ok(Ok(pool)) => pool,
            Ok(Err(error)) => {
                tracing::warn!(community = community_id, %error, "own op pages not re-warmed: no record pool");
                return;
            }
            Err(error) => {
                tracing::warn!(community = community_id, %error, "own op pages not re-warmed: no transport");
                return;
            }
        };
        match local_pages_after(&*pool, &record, GOV_HEAD, floor).await {
            Ok(pages) => {
                for page in &pages {
                    self.ctx.communities.note_own_op_page(community_id, page);
                }
                tracing::debug!(
                    community = community_id,
                    pages = pages.len(),
                    floor,
                    "own op pages re-warmed"
                );
            }
            Err(error) => {
                tracing::warn!(community = community_id, %error, "own op pages not re-warmed: local op log unreadable");
            }
        }
    }
}

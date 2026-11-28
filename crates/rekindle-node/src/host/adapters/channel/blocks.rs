//! The community members we block, by pseudonym (plan E3.1 F3, D26): the
//! `community_blocks` rows the ingest gate's `BlockList` is read from. An
//! identity-key block cannot match a community post without linking the
//! author's pseudonyms (threat model I8, L1, L3), so a community block names
//! the pseudonym. The block is ours alone; no member is told.

use rekindle_channel::error::ChannelError;
use rekindle_channel::ChannelMessagingDeps as _;
use rekindle_db::repo::community_blocks;

use super::HostChannel;

impl HostChannel {
    /// Block `pseudonym` in `community_id`.
    ///
    /// # Errors
    /// Our own pseudonym, no identity unlocked, or the write failed.
    pub(crate) async fn block_member(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Result<(), ChannelError> {
        let pseudonym = pseudonym.to_ascii_lowercase();
        if self
            .my_pseudonym_hex(community_id)
            .is_some_and(|mine| mine.eq_ignore_ascii_case(&pseudonym))
        {
            return Err(ChannelError::PermissionDenied(
                "we cannot block ourselves".into(),
            ));
        }
        let (owner, db) = self.owner_and_db()?;
        let community = community_id.to_owned();
        let at = self.ctx().clock.now_ms_i64();
        db.call(move |conn| community_blocks::insert(conn, &owner, &community, &pseudonym, at))
            .await
            .map_err(|e| ChannelError::Adapter(format!("store: {e}")))
    }

    /// Lift the block on `pseudonym` in `community_id`.
    ///
    /// # Errors
    /// No identity unlocked, or the write failed.
    pub(crate) async fn unblock_member(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Result<(), ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let (community, pseudonym) = (community_id.to_owned(), pseudonym.to_owned());
        db.call(move |conn| community_blocks::delete(conn, &owner, &community, &pseudonym))
            .await
            .map(|_| ())
            .map_err(|e| ChannelError::Adapter(format!("store: {e}")))
    }

    /// The pseudonyms we block in `community_id`, newest first.
    ///
    /// # Errors
    /// No identity unlocked, or the read failed.
    pub(crate) async fn blocked_members(
        &self,
        community_id: &str,
    ) -> Result<Vec<String>, ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let community = community_id.to_owned();
        db.call(move |conn| community_blocks::list(conn, &owner, &community))
            .await
            .map(|rows| rows.into_iter().map(|b| b.pseudonym).collect())
            .map_err(|e| ChannelError::Adapter(format!("store: {e}")))
    }
}

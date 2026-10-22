//! A community's name, description, icon and banner on the Host (plan E3.1
//! D1), both modes: one `CommunityMeta` entry that carries every field, the
//! unchanged ones from merged governance, so editing one keeps the others.
//! The merge projects it into the store and journals `MetadataChanged`;
//! the row follows here.

use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::governance::GovernanceEntry;

use super::HostGovernance;

/// The fields an edit sets; `None` keeps the merged value.
#[derive(Debug, Clone, Default)]
pub(crate) struct InfoEdit {
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon_hash: Option<String>,
    pub banner_hash: Option<String>,
}

impl HostGovernance {
    /// Write `edit` over `community`'s merged metadata.
    ///
    /// # Errors
    /// The community has no merged governance, we may not manage it, or a
    /// write failed.
    pub(crate) async fn update_info(&self, community: &str, edit: InfoEdit) -> Result<(), String> {
        let current = self
            .ctx
            .communities
            .governance_state(community)
            .ok_or("the community's governance is not loaded")?
            .metadata;
        let (name, description, icon_hash, banner_hash) = current.map_or_else(
            || (None, None, None, None),
            |m| (Some(m.name), m.description, m.icon_hash, m.banner_hash),
        );
        let lamport = self
            .next_governance_lamport(community)
            .map_err(|e| e.to_string())?;
        rekindle_governance_runtime::apply::write_entry(
            self,
            community,
            GovernanceEntry::CommunityMeta {
                name: edit.name.clone().or(name),
                description: edit.description.clone().or(description),
                icon_hash: edit.icon_hash.clone().or(icon_hash),
                banner_hash: edit.banner_hash.clone().or(banner_hash),
                lamport,
            },
        )
        .await
        .map_err(|e| format!("community info: {e}"))?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let cid = community.to_owned();
        db.call(move |conn| {
            use rekindle_db::repo::communities::{set, Column};
            let fields = [
                (Column::Name, edit.name),
                (Column::Description, edit.description),
                (Column::IconHash, edit.icon_hash),
                (Column::BannerHash, edit.banner_hash),
            ];
            for (column, value) in fields {
                if let Some(value) = value {
                    set(conn, &owner, &cid, column, value)?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| format!("community info written but not stored: {e}"))
    }
}

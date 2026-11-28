//! A successor's sealed invites on the DHT (plan E3.2 SS3c, lead decision
//! (34)). The publisher reads each continuing member's card `hpkeKey`
//! ([`continuing_cards`]), seals one invite to the new community to each
//! (`rekindle_codec::successor_invites`) and writes them into sealed DFLT(1)
//! pages, each written once with a fresh owner that is then dropped (ADR
//! 0011 "Sealed pages", as `checkpointing::seal_pages`): [`write_invites`]
//! returns the first page's key, the successor op's `invitesRecord`. A
//! follower finds its own entry by its old-community pseudonym and opens it
//! with its card HPKE secret ([`open_own_invite`]); a member the publisher
//! did not continue has no entry and is told so.

use rekindle_codec::successor_invites::{
    decode_invites_page, encode_invites_page, open_invite, paginate, InvitesPage, SealedInvite,
};
use rekindle_governance::state::GovernanceState;
use rekindle_records::port::OpenIntent;
use rekindle_records::schema;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::member_card::{read_card, MemberRecordRef};

/// The members a successor continues by default: the roster at publish,
/// without departed (left, removed or banned) members and without `me`.
#[must_use]
pub fn default_continuing(state: &GovernanceState, me: &PseudonymKey) -> Vec<PseudonymKey> {
    let mut members: Vec<_> = (state.members.keys())
        .filter(|p| *p != me && !state.is_departed(p))
        .cloned()
        .collect();
    members.sort_by_key(|p| p.0);
    members
}

/// Each of `chosen`'s card `hpkeKey`, read from its member record pinned to
/// the owner `MemberJoined` names.
///
/// # Errors
/// A chosen key is not a current member, or its card cannot be read or has
/// none: the publish names that member and writes nothing.
pub async fn continuing_cards<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    state: &GovernanceState,
    chosen: &[PseudonymKey],
) -> Result<Vec<(PseudonymKey, [u8; 32])>, GovernanceRuntimeError> {
    let port = deps.record_port()?;
    let mut cards = Vec::with_capacity(chosen.len());
    for pseudonym in chosen {
        let member = (state.members.get(pseudonym))
            .filter(|_| !state.is_departed(pseudonym))
            .ok_or_else(|| {
                GovernanceRuntimeError::OpRefused(format!(
                    "{} is not a current member and cannot be continued",
                    pseudonym.to_hex()
                ))
            })?;
        let member_ref = MemberRecordRef {
            community_id,
            pseudonym,
            record: &member.member_record,
            owner: &member.member_record_owner,
        };
        let card = read_card(port.as_ref(), &member_ref).await;
        let card = card.ok().flatten().ok_or_else(|| {
            GovernanceRuntimeError::OpRefused(format!(
                "member {} has no readable card to seal its invite to",
                pseudonym.to_hex()
            ))
        })?;
        cards.push((pseudonym.clone(), card.card.hpke_key));
    }
    Ok(cards)
}

/// Write `sealed` as sealed pages: the further pages first, then the first
/// page listing their keys. Returns the first page's key.
///
/// # Errors
/// Too many members for the pages, or a page write failed.
pub async fn write_invites<D: GovernanceRuntimeDeps>(
    deps: &D,
    sealed: Vec<SealedInvite>,
) -> Result<String, GovernanceRuntimeError> {
    let mut pages =
        paginate(&sealed).map_err(|e| GovernanceRuntimeError::OpRefused(e.to_string()))?;
    let first = pages.remove(0);
    let mut keys = Vec::with_capacity(pages.len());
    for page in &pages {
        keys.push(write_page(deps, encode_invites_page(page)?).await?);
    }
    let first = InvitesPage {
        pages: keys,
        entries: first.entries,
    };
    write_page(deps, encode_invites_page(&first)?).await
}

/// Create a DFLT(1) page, write it once and drop its owner keypair, which
/// is never stored.
async fn write_page<D: GovernanceRuntimeDeps>(
    deps: &D,
    bytes: Vec<u8>,
) -> Result<String, GovernanceRuntimeError> {
    let record = deps.create_record(&schema::sealed_page()).await?;
    let outcome = (deps.set_dht_value(record.lease, 0, bytes, record.owner_keypair.clone())).await;
    deps.release_record(record.lease).await;
    if let Some(stale) = outcome? {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }
    Ok(record.record_key)
}

async fn read_page<D: GovernanceRuntimeDeps>(
    deps: &D,
    key: &str,
) -> Result<InvitesPage, GovernanceRuntimeError> {
    let lease = deps.acquire_record(key, None, OpenIntent::Check).await?;
    let read = deps.get_dht_value(lease, 0, true).await;
    deps.release_record(lease).await;
    let bytes = read?.ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!("the successor invites page {key} holds nothing"))
    })?;
    decode_invites_page(&bytes).map_err(|e| GovernanceRuntimeError::Encoding(e.to_string()))
}

/// Our own invite to `new_anchor` from the invites at `record`: the entry
/// for `me` (our pseudonym in `old_anchor`), opened with our card HPKE
/// secret.
///
/// # Errors
/// A page cannot be read, the publisher did not continue us (no entry
/// names us), or our entry does not open.
pub async fn open_own_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    old_anchor: &str,
    new_anchor: &str,
    record: &str,
    me: &PseudonymKey,
    hpke_secret: &[u8; 32],
) -> Result<String, GovernanceRuntimeError> {
    let follow = Follow {
        old_anchor,
        new_anchor,
        me,
        hpke_secret,
    };
    follow
        .open(record, |key| async move { read_page(deps, &key).await })
        .await
}

/// One follower opening its own invite from the pages `read` returns.
struct Follow<'a> {
    old_anchor: &'a str,
    new_anchor: &'a str,
    me: &'a PseudonymKey,
    hpke_secret: &'a [u8; 32],
}

impl Follow<'_> {
    async fn open<R, F>(&self, record: &str, read: R) -> Result<String, GovernanceRuntimeError>
    where
        R: Fn(String) -> F,
        F: std::future::Future<Output = Result<InvitesPage, GovernanceRuntimeError>>,
    {
        let first = read(record.to_owned()).await?;
        let mut entry = find(&first, self.me);
        for key in &first.pages {
            if entry.is_some() {
                break;
            }
            entry = find(&read(key.clone()).await?, self.me);
        }
        let new_anchor = self.new_anchor;
        let entry = entry.ok_or_else(|| {
            GovernanceRuntimeError::OpRefused(format!(
                "the publisher of {new_anchor} did not continue you: its sealed invites hold none for you"
            ))
        })?;
        open_invite(self.old_anchor, new_anchor, &entry, self.hpke_secret)
            .map_err(|e| GovernanceRuntimeError::Crypto(format!("your successor invite: {e}")))
    }
}

fn find(page: &InvitesPage, me: &PseudonymKey) -> Option<SealedInvite> {
    page.entries.iter().find(|e| e.member == *me).cloned()
}

#[cfg(test)]
#[path = "successor_invites_tests.rs"]
mod tests;

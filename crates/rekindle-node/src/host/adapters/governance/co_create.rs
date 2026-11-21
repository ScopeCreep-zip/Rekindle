//! The creator-set round on the Host (plan E3.2 S7s.3, decisions (16),
//! (17), (20)). The creator reserves the community's records (the anchor
//! key is then known), writes a `pending_creations` row per named
//! co-creator, holds the reservation in memory (`A`'s one-time key is never
//! stored, ADR 0011) and sends each a `CreatorInvite`. The genesis is
//! written once every named co-creator has accepted, declined or been
//! removed, listing only the verified accepts; each accepted co-creator is
//! then sent an invite link (`CreatorJoin`) and joins through the normal
//! join, holding creator rights from genesis. A cancel releases the
//! reservation and withdraws every invite. The co-creator answers a held
//! invite with its pseudonym for the new anchor, signed, or a decline.

use std::collections::{BTreeSet, HashMap};

use rekindle_crypto::group::pseudonym::{derive_community_pseudonym, sign_with_pseudonym};
use rekindle_governance_runtime::{CoCreatorAcceptance, ReservedCommunity};
use rekindle_transport::operations::friend::creator_round::{self, CreatorFrame};
use rekindle_transport::operations::friend::CreatorAnswer;
use rekindle_transport::operations::friend::FriendHost as _;
use rekindle_transport::TransportNode;
use rekindle_types::dto::community::{
    CoCreationSnapshot, CoCreatorState, CommunityCreated, HeldCreatorInvite, PendingCoCreation,
};
use rekindle_types::governance::{AdmissionMode, MAX_ADDITIONAL_CREATORS};
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{FriendEvent, SubscriptionEvent};

use super::create_leave::NOT_CONNECTED;
use super::HostGovernance;
use crate::host::adapters::friends::creator_round::status;
use crate::host::adapters::friends::HostFriends;

/// A reserved creation waiting on its co-creators.
pub(crate) struct PendingCreation {
    reserved: ReservedCommunity,
    name: String,
    description: String,
    admission: AdmissionMode,
    /// Every co-creator named, in the order named.
    named: Vec<String>,
    /// The named co-creators that have neither answered nor been removed.
    waiting: BTreeSet<String>,
}

/// Our reserved creations by anchor; memory only, released at lock (the
/// rows left behind are withdrawn at the next unlock, decision (20)).
#[derive(Default)]
pub(crate) struct PendingCreations(parking_lot::Mutex<HashMap<String, PendingCreation>>);

impl PendingCreations {
    /// Drop every reservation (lock): their one-time keys go with them.
    pub(crate) fn clear(&self) {
        self.0.lock().clear();
    }

    /// Take every reservation out (lock) and release its record leases on
    /// the record pool (decision (23)); `A`'s one-time key is dropped with
    /// it. Returns how many reservations were released.
    pub(crate) async fn release_all(&self, transport: &TransportNode) -> usize {
        let taken: Vec<PendingCreation> = self.0.lock().drain().map(|(_, p)| p).collect();
        let count = taken.len();
        for pending in taken {
            for lease in pending.reserved.into_leases() {
                rekindle_transport::broadcast::dht_writes::release(transport, lease).await;
            }
        }
        count
    }
}

/// `peer`'s state in `pending`, in `FriendEvent::CoCreatorStatus` words:
/// waited on, answered (a verified accept or a decline), or removed.
fn co_creator_status(
    pending: &PendingCreation,
    answers: Option<&HashMap<String, CreatorAnswer>>,
    peer: &str,
) -> &'static str {
    if pending.waiting.contains(peer) {
        return "pending";
    }
    match answers.and_then(|a| a.get(peer)) {
        Some(Some(_)) => "accepted",
        Some(None) => "declined",
        None => "removed",
    }
}

/// `co_creators` as a set of distinct friends other than `me`, refused
/// when malformed.
fn named_co_creators(
    friends: &HostFriends,
    me: &str,
    co_creators: &[String],
) -> Result<Vec<String>, String> {
    if co_creators.len() > MAX_ADDITIONAL_CREATORS {
        return Err(format!(
            "at most {MAX_ADDITIONAL_CREATORS} co-creators can be named"
        ));
    }
    let mut named: Vec<String> = Vec::with_capacity(co_creators.len());
    for peer in co_creators {
        if peer == me || named.contains(peer) {
            return Err(format!("co-creator {peer} named twice or is us"));
        }
        if !creator_round::is_accepted_friend(friends, peer) {
            return Err(format!(
                "{peer} is not an accepted friend: only friends can be named co-creators"
            ));
        }
        named.push(peer.clone());
    }
    Ok(named)
}

impl HostGovernance {
    /// Start creating `name` with `co_creators` (friends): reserve, record
    /// and invite. The community is written later, when the round settles
    /// ([`Self::co_creator_settled`]).
    ///
    /// # Errors
    /// A co-creator is not a friend (or named twice, or us, or too many),
    /// the network is not ready, the records or rows could not be made, or
    /// an invite could not be committed (the creation is then cancelled).
    pub(crate) async fn create_with_co_creators(
        &self,
        name: &str,
        description: &str,
        admission: AdmissionMode,
        co_creators: &[String],
    ) -> Result<CommunityCreated, String> {
        let me = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let friends = HostFriends::of(&self.ctx);
        let named = named_co_creators(&friends, &me, co_creators)?;
        if !self.transport().is_ok_and(|node| node.is_ready()) {
            return Err(NOT_CONNECTED.into());
        }
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let reserved = rekindle_governance_runtime::reserve_community(self)
            .await
            .map_err(|e| format!("community create failed: {e}"))?;
        let anchor = reserved.anchor_key().to_owned();
        let (owner, a, rows) = (me.clone(), anchor.clone(), named.clone());
        if let Err(e) = db
            .call(move |c| rekindle_db::repo::creator_round::put_pending(c, &owner, &a, &rows))
            .await
        {
            rekindle_governance_runtime::release_reserved(self, reserved).await;
            return Err(format!("co-creators not recorded: {e}"));
        }
        self.ctx.pending_creations.0.lock().insert(
            anchor.clone(),
            PendingCreation {
                reserved,
                name: name.to_owned(),
                description: description.to_owned(),
                admission,
                waiting: named.iter().cloned().collect(),
                named: named.clone(),
            },
        );
        let invite = CreatorFrame::Invite {
            anchor: anchor.clone(),
            community_name: name.to_owned(),
        };
        for (i, peer) in named.iter().enumerate() {
            if let Err(e) = creator_round::send(&friends, peer, &invite).await {
                self.abandon(&anchor, &named[..i]).await;
                return Err(format!("co-creator invite to {peer} not sent: {e}"));
            }
            status(&friends, peer, &anchor, "pending");
        }
        Ok(CommunityCreated {
            anchor,
            registry_key: String::new(),
            name: name.to_owned(),
            pending_co_creators: named,
        })
    }

    /// Remove `peer`, which has not answered, from our creation of
    /// `anchor`: it is sent `CreatorWithdrawn` and the creation no longer
    /// waits on it.
    ///
    /// # Errors
    /// The creation does not wait on `peer`, or the withdrawal or the row
    /// update failed.
    pub(crate) async fn remove_co_creator(&self, anchor: &str, peer: &str) -> Result<(), String> {
        let waits = (self.ctx.pending_creations.0.lock())
            .get(anchor)
            .is_some_and(|p| p.waiting.contains(peer));
        if !waits {
            return Err(format!("the creation of {anchor} does not wait on {peer}"));
        }
        let friends = HostFriends::of(&self.ctx);
        let withdrawn = CreatorFrame::Withdrawn {
            anchor: anchor.to_owned(),
        };
        creator_round::send(&friends, peer, &withdrawn)
            .await
            .map_err(|e| format!("CreatorWithdrawn not committed: {e}"))?;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let (a, p) = (anchor.to_owned(), peer.to_owned());
        db.call(move |c| rekindle_db::repo::creator_round::remove_pending(c, &owner, &a, &p))
            .await
            .map_err(|e| format!("co-creator row not removed: {e}"))?;
        status(&friends, peer, anchor, "removed");
        self.co_creator_settled(anchor, peer).await;
        Ok(())
    }

    /// Cancel our creation of `anchor`: the reservation is released and
    /// every named co-creator is sent `CreatorWithdrawn`.
    ///
    /// # Errors
    /// No creation of `anchor` is pending.
    pub(crate) async fn cancel_creation(&self, anchor: &str) -> Result<(), String> {
        let named = (self.ctx.pending_creations.0.lock())
            .get(anchor)
            .map(|p| p.named.clone())
            .ok_or_else(|| format!("no creation of {anchor} is pending"))?;
        self.abandon(anchor, &named).await;
        Ok(())
    }

    /// Release the reservation of `anchor`, delete its rows and answers
    /// and withdraw the invites sent to `invited`.
    async fn abandon(&self, anchor: &str, invited: &[String]) {
        let pending = self.ctx.pending_creations.0.lock().remove(anchor);
        if let Some(pending) = pending {
            rekindle_governance_runtime::release_reserved(self, pending.reserved).await;
        }
        self.ctx.friend_stores.creator_answers.lock().remove(anchor);
        self.clear_pending_rows(anchor).await;
        self.withdraw(anchor, invited).await;
    }

    /// Send each of `peers` `CreatorWithdrawn` for `anchor`.
    async fn withdraw(&self, anchor: &str, peers: &[String]) {
        let friends = HostFriends::of(&self.ctx);
        let frame = CreatorFrame::Withdrawn {
            anchor: anchor.to_owned(),
        };
        for peer in peers {
            match creator_round::send(&friends, peer, &frame).await {
                Ok(_) => status(&friends, peer, anchor, "withdrawn"),
                Err(e) => tracing::warn!(error = %e, "CreatorWithdrawn not committed"),
            }
        }
    }

    async fn clear_pending_rows(&self, anchor: &str) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            tracing::warn!("co-creator rows not cleared: locked");
            return;
        };
        let a = anchor.to_owned();
        if let Err(e) = db
            .call(move |c| rekindle_db::repo::creator_round::clear_pending(c, &owner, &a))
            .await
        {
            tracing::warn!(error = %e, "co-creator rows not cleared");
        }
    }

    /// `peer` answered, or was removed from, our creation of `anchor`;
    /// once none is left waiting, write the genesis with the verified
    /// accepts and send each accepted co-creator its join link.
    pub(crate) async fn co_creator_settled(&self, anchor: &str, peer: &str) {
        let pending = {
            let mut map = self.ctx.pending_creations.0.lock();
            let done = map.get_mut(anchor).is_some_and(|p| {
                p.waiting.remove(peer);
                p.waiting.is_empty()
            });
            if done {
                map.remove(anchor)
            } else {
                None
            }
        };
        let Some(pending) = pending else { return };
        let answers = (self.ctx.friend_stores.creator_answers.lock())
            .remove(anchor)
            .unwrap_or_default();
        let (mut joiners, mut accepts) = (Vec::new(), Vec::new());
        for named in &pending.named {
            if let Some(Some((pseudonym, signature))) = answers.get(named) {
                joiners.push(named.clone());
                accepts.push(CoCreatorAcceptance {
                    pseudonym: PseudonymKey(*pseudonym),
                    signature: *signature,
                });
            }
        }
        let PendingCreation {
            reserved,
            name,
            description,
            admission,
            ..
        } = pending;
        let finished = rekindle_governance_runtime::finish_community(
            self, reserved, &name, admission, &accepts,
        )
        .await
        .map_err(|e| format!("community create failed: {e}"));
        let created = match finished {
            Ok(id) => self.store_created(id, &name, &description).await,
            Err(e) => Err(e),
        };
        self.clear_pending_rows(anchor).await;
        let written = created.is_ok();
        match created {
            Ok(created) => {
                self.activate_created(&created.anchor).await;
                self.send_join_links(&created.anchor, &joiners).await;
            }
            Err(e) => {
                tracing::warn!(community = %anchor, error = %e, "co-created community not written");
                self.withdraw(anchor, &joiners).await;
            }
        }
        HostFriends::of(&self.ctx).publish(SubscriptionEvent::Friend(
            FriendEvent::CoCreationFinished {
                anchor: anchor.to_owned(),
                created: written,
            },
        ));
    }

    /// Mint each accepted co-creator a one-use invite and send it as
    /// `CreatorJoin` (decision (17): it joins through the normal join).
    async fn send_join_links(&self, anchor: &str, joiners: &[String]) {
        let friends = HostFriends::of(&self.ctx);
        for peer in joiners {
            let invite = match self.create_invite(anchor, 1, None).await {
                Ok(invite) => invite,
                Err(e) => {
                    tracing::warn!(error = ?e, "co-creator join link not minted");
                    status(&friends, peer, anchor, "withdrawn");
                    continue;
                }
            };
            let frame = CreatorFrame::Join {
                anchor: anchor.to_owned(),
                invite: invite.url,
            };
            match creator_round::send(&friends, peer, &frame).await {
                Ok(_) => status(&friends, peer, anchor, "joining"),
                Err(e) => tracing::warn!(error = %e, "CreatorJoin not committed"),
            }
        }
    }

    /// The held co-creator invites (`creator_invites`, as the unlock replay
    /// states them) and each creation of ours still waiting, with every
    /// named co-creator's state (plan E3.2 S7s.3g, lead decision (33)).
    /// The creations come from the in-memory reservations alone: a
    /// `pending_creations` row without one is a dead creation the unlock
    /// sweep has already withdrawn (decision (20)), never a live one.
    ///
    /// # Errors
    /// No identity is unlocked or the invites could not be read.
    pub(crate) async fn co_creation_snapshot(&self) -> Result<CoCreationSnapshot, String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let rows = db
            .call(move |c| rekindle_db::repo::creator_round::invites(c, &owner))
            .await
            .map_err(|e| format!("co-creator invites not read: {e}"))?;
        let invites = rows
            .into_iter()
            .map(|row| HeldCreatorInvite {
                anchor: row.anchor,
                peer_key: row.inviter_key,
                community_name: row.community_name,
            })
            .collect();
        let answers = self.ctx.friend_stores.creator_answers.lock().clone();
        let creations = (self.ctx.pending_creations.0.lock())
            .iter()
            .map(|(anchor, pending)| PendingCoCreation {
                anchor: anchor.clone(),
                name: pending.name.clone(),
                co_creators: pending
                    .named
                    .iter()
                    .map(|peer| CoCreatorState {
                        peer_key: peer.clone(),
                        status: co_creator_status(pending, answers.get(anchor), peer).into(),
                    })
                    .collect(),
            })
            .collect();
        Ok(CoCreationSnapshot { invites, creations })
    }

    /// Answer the held co-creator invite to `anchor`: an accept sends our
    /// pseudonym for that anchor, signed (`CoCreatorAcceptance`), and keeps the
    /// invite (accepted) for the creator's join link; a decline sends
    /// `CreatorDecline` and drops it.
    ///
    /// # Errors
    /// No invite to `anchor` is held, or the answer was not committed.
    pub(crate) async fn answer_creator_invite(
        &self,
        anchor: &str,
        accept: bool,
    ) -> Result<(), String> {
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "no identity is unlocked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let (o, a) = (owner.clone(), anchor.to_owned());
        let row = db
            .call(move |c| rekindle_db::repo::creator_round::invite(c, &o, &a))
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no co-creator invite to {anchor} is held"))?;
        let friends = HostFriends::of(&self.ctx);
        let frame = if accept {
            let secret = self
                .ctx
                .identity_secret()
                .ok_or("no identity is unlocked")?;
            let signing = derive_community_pseudonym(&secret, anchor);
            let pseudonym = PseudonymKey(signing.verifying_key().to_bytes());
            let signing_bytes =
                rekindle_governance::genesis::creator_accept_signing_bytes(anchor, &pseudonym);
            CreatorFrame::Accept {
                anchor: anchor.to_owned(),
                pseudonym: pseudonym.0,
                signature: sign_with_pseudonym(&signing, &signing_bytes),
            }
        } else {
            CreatorFrame::Decline {
                anchor: anchor.to_owned(),
            }
        };
        creator_round::send(&friends, &row.inviter_key, &frame)
            .await
            .map_err(|e| format!("co-creator answer not committed: {e}"))?;
        let (a, inviter) = (anchor.to_owned(), row.inviter_key);
        db.call(move |c| {
            if accept {
                rekindle_db::repo::creator_round::mark_accepted(c, &owner, &a, &inviter)
            } else {
                rekindle_db::repo::creator_round::take_invite(c, &owner, &a, &inviter)
                    .map(|taken| taken.is_some())
            }
        })
        .await
        .map_err(|e| format!("co-creator answer sent, not stored: {e}"))?;
        Ok(())
    }
}

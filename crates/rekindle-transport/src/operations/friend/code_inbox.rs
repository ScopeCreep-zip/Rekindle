//! A friend code's request inbox (plan RV2e, lead decision 67: every first
//! contact has a durable copy). A request sent with our friend code goes by
//! `app_call` **and** is written once into this inbox, the same datagram
//! both ways (`dm::first_contact_durable`), as an invite's redemption is
//! written into its inbox and an accept into the requester's reply inbox
//! (RV2b). When the call path to us stalls, the write still reaches our
//! watch.
//!
//! The code itself is unchanged (199 bytes at most,
//! `rekindle_types::friend_code`): the inbox's one-time writer is derived
//! from the code's token (`rekindle_secrets::friend_token::inbox_writer`),
//! so exactly the code's holders can write it (Veilid refuses any other
//! writer, `storage_manager/schema.rs` `check_subkey_value_data`), and the
//! inbox's record key is published in our profile's `CODE_INBOX` subkey,
//! the profile the code names.
//!
//! A code is reusable while its token lives, but an inbox slot takes one
//! write, so each slot serves one request: once a write lands, the slot is
//! rolled (a fresh record under a fresh owner key, the same derived writer,
//! published in its place, the old one released). A second redeemer racing
//! the first finds the slot taken, its call decides that attempt, and its
//! next attempt reads the rolled slot. Rotating the token rolls the slot
//! too: the old code's holders can no longer derive the writer.

use rekindle_codec::capnp_codec::direct::{DirectBody, DmTypeId, RequestAuth};
use rekindle_types::dht_layout::profile;
use rekindle_types::key_format::{self, FriendRequestToken};

use super::error::FriendError;
use super::host::FriendHost;
use super::{inbox, state};

/// Our current token.
async fn token(host: &dyn FriendHost) -> Result<FriendRequestToken, FriendError> {
    let owner = state::owner(host)?;
    let token = state::db_call!(host, "read friend token", move |conn| {
        rekindle_db::repo::identity::friend_request_token(conn, &owner)
    })
    .await?
    .ok_or(FriendError::Locked)?;
    key_format::friend_request_token(&token).map_err(|e| FriendError::store("friend token", e))
}

/// Our code's current inbox, if a code was issued.
async fn current(host: &dyn FriendHost) -> Result<Option<String>, FriendError> {
    let owner = state::owner(host)?;
    state::db_call!(host, "read code inbox", move |conn| {
        rekindle_db::repo::outgoing_invites::code_inbox_key(conn, &owner)
    })
    .await
}

/// Make sure our live code has its inbox. The token, and so the code, is
/// live from the identity's creation (`friend_token::mint`), so the Host
/// issues the slot once our profile is held (the own-records stage), not
/// when the code is first shown.
///
/// # Errors
/// The inbox could not be created, opened, kept or published.
pub async fn issue(host: &dyn FriendHost) -> Result<(), FriendError> {
    if current(host).await?.is_some() {
        return Ok(());
    }
    roll(host).await
}

/// A fresh slot for our current token: created, put on the network, held
/// and watched, published in our profile, then recorded; the slot it
/// replaces is released and its owner keypair forgotten.
///
/// # Errors
/// The slot could not be created, opened, kept or published.
pub(crate) async fn roll(host: &dyn FriendHost) -> Result<(), FriendError> {
    roll_for(host, &token(host).await?).await
}

/// [`roll`] for `token`: a token rotation rolls the slot to the new token's
/// writer before the token is stored, so a stored token always has its
/// slot.
///
/// # Errors
/// As [`roll`].
pub(crate) async fn roll_for(
    host: &dyn FriendHost,
    token: &FriendRequestToken,
) -> Result<(), FriendError> {
    let signing = rekindle_secrets::friend_token::inbox_writer(token);
    let (record_key, owner_keypair, lease, _writer) = inbox::new_inbox_for(host, &signing).await?;
    inbox::open(host, lease).await?;
    host.keep_code_inbox(&record_key, &owner_keypair, lease)
        .await?;
    if let Err(e) = host.publish_code_inbox(&record_key).await {
        host.drop_code_inbox(&record_key).await;
        return Err(e);
    }
    let (owner, key) = (state::owner(host)?, record_key.clone());
    let old = state::db_call!(host, "store code inbox", move |conn| {
        let old = rekindle_db::repo::outgoing_invites::code_inbox_key(conn, &owner)?;
        rekindle_db::repo::outgoing_invites::set_code_inbox_key(conn, &owner, &key)?;
        Ok(old)
    })
    .await?;
    if let Some(old) = old.filter(|old| *old != record_key) {
        host.drop_code_inbox(&old).await;
    }
    tracing::info!("friend code inbox rolled");
    Ok(())
}

/// What [`landed`] came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRoll {
    /// The slot was rolled, or was already replaced (nothing to do).
    Done,
    /// The roll budget is spent: the slot stays taken (a holder's request
    /// still goes by call) and the roll is attempted again later.
    Paced,
    /// The roll failed; attempted again later.
    Missed,
}

/// A write landed in our code's slot `record_key` and was opened: roll it,
/// once (the slot holds one write; a slot already replaced is not rolled
/// again), if `spend` grants a roll. Each roll costs a record create and
/// two writes, and any holder of the code can force one with a junk
/// write, so the Host spends a token of a route-style budget per roll
/// (`rekindle_route::budget::RouteBudget`).
pub async fn landed(
    host: &dyn FriendHost,
    record_key: &str,
    spend: impl FnOnce() -> bool,
) -> CodeRoll {
    match current(host).await {
        Ok(Some(key)) if key == record_key => {
            if !spend() {
                tracing::warn!("friend code inbox roll budget spent — the slot stays taken");
                return CodeRoll::Paced;
            }
            match roll(host).await {
                Ok(()) => CodeRoll::Done,
                Err(e) => {
                    tracing::warn!(error = %e, "friend code inbox not rolled");
                    CodeRoll::Missed
                }
            }
        }
        Ok(_) => CodeRoll::Done,
        Err(e) => {
            tracing::warn!(error = %e, "friend code inbox not read");
            CodeRoll::Missed
        }
    }
}

/// The slot a request row to `to` is also written into (requester side):
/// a request sent with `to`'s friend code, whose profile publishes a slot;
/// its record key and the writer derived from the code's token. Read from
/// the network at each attempt, so a rolled slot is found. `None`: not a
/// code's request, or `to` published no slot (no slot was issued with the
/// code; its request goes by `app_call`, the only path it has).
pub(crate) async fn slot(
    host: &dyn FriendHost,
    to: &str,
    framed: &crate::operations::dm::Framed,
) -> Option<(String, String)> {
    let crate::operations::dm::Framed::Sealed { type_id, body, .. } = framed else {
        return None;
    };
    if *type_id != DmTypeId::FriendRequest.as_u16() {
        return None;
    }
    let DirectBody::FriendRequest(request) =
        DirectBody::decode(DmTypeId::FriendRequest, body).ok()?
    else {
        return None;
    };
    let RequestAuth::Token(token) = request.auth else {
        return None;
    };
    let token = key_format::friend_request_token(&token).ok()?;
    let profile_key = state::friend(host, to)?.dht_record_key?;
    let records = host.node().ok()?.require_records().ok()?;
    let published = rekindle_protocol::dht::profile::read_profile_subkey(
        records.as_ref(),
        &profile_key,
        profile::CODE_INBOX,
        true,
    )
    .await;
    let published = match published {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!(error = %e, "friend code inbox not read — this attempt goes by call only");
            return None;
        }
    };
    let record_key = key_format::record_key(std::str::from_utf8(&published).ok()?).ok()?;
    let signing = rekindle_secrets::friend_token::inbox_writer(&token);
    let writer = rekindle_crypto::keypair_text::keypair_text_from_signing(&signing);
    Some((record_key.to_string(), writer))
}

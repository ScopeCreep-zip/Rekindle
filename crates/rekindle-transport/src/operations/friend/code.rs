//! Our friend code (plan E2.5, D17): `rekindle://add/<identity key>/
//! <profile record key>/<request token>` and its QR. Rotating the token
//! (`FriendTokenRotate`) stops requests sent with any code already shared
//! without touching a friendship.

use rekindle_types::dto::friends::FriendCodeView;
use rekindle_types::friend_code::FriendCode;
use rekindle_types::key_format::{self, FriendRequestToken};

use super::error::FriendError;
use super::host::FriendHost;
use super::invites::published_identity;
use super::state::{self, OwnIdentity};

/// Our friend code, once our profile is published (the code names it).
///
/// # Errors
/// `Locked`; `NotReady` while our profile is not published; the token
/// could not be read.
pub async fn own(host: &dyn FriendHost) -> Result<FriendCodeView, FriendError> {
    let own = published_identity(host).await?;
    let owner = own.public_key.clone();
    let token = state::db_call!(host, "read friend token", move |conn| {
        rekindle_db::repo::identity::friend_request_token(conn, &owner)
    })
    .await?
    .ok_or(FriendError::Locked)?;
    let token = key_format::friend_request_token(&token)
        .map_err(|e| FriendError::store("friend token", e))?;
    view(&own, token)
}

/// Mint a new friend-request token and store it: codes carrying the old
/// one stop working. Answers the new code.
///
/// # Errors
/// `Locked`; `NotReady` while our profile is not published; the write
/// failed.
pub async fn rotate_token(host: &dyn FriendHost) -> Result<FriendCodeView, FriendError> {
    let own = published_identity(host).await?;
    let token = rekindle_secrets::friend_token::mint();
    let (owner, spelled) = (own.public_key.clone(), token.as_str().to_string());
    state::db_call!(host, "rotate friend token", move |conn| {
        rekindle_db::repo::identity::set_friend_request_token(conn, &owner, &spelled)
    })
    .await?;
    view(&own, token)
}

/// The code `own` and `token` make, with its QR.
fn view(own: &OwnIdentity, token: FriendRequestToken) -> Result<FriendCodeView, FriendError> {
    let code = FriendCode {
        public_key: key_format::public_key_hex(&own.public_key)
            .map_err(|e| FriendError::store("our identity key", e))?,
        profile_key: key_format::record_key(&own.profile_dht_key)
            .map_err(|e| FriendError::store("our profile key", e))?,
        token,
    };
    let url = code.to_url();
    Ok(FriendCodeView {
        qr_rows: super::qr::rows(url.as_bytes(), "friend code")?,
        public_key: code.public_key.to_string(),
        profile_key: code.profile_key.to_string(),
        token: code.token.as_str().to_string(),
        url,
    })
}

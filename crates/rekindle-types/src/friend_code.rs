//! Friend codes (plan E2.5, D17): `rekindle://add/<identity public key>/
//! <profile record key>/<request token>`. A QR encodes the same string.
//!
//! The code is everything a requester needs: the target's identity key
//! (the HPKE recipient of the sealed `FriendRequest`), its profile record
//! (whose route subkey the request is sent to) and the request token the
//! target checks. Rotating the token stops requests sent with the old code
//! without touching existing friends.

use crate::key_format::{self, FriendRequestToken, KeyFormatError, PublicKeyHex, RecordKeyStr};

/// The friend-code scheme and path.
pub const FRIEND_CODE_PREFIX: &str = "rekindle://add/";

/// Longest friend code accepted: the prefix (15), a 64-hex key, a record
/// key with its encryption segment (`VLD0:` + 43 + `:` + 43 = 92), a
/// 26-character token and three separators make 199 bytes.
pub const MAX_FRIEND_CODE_URL_LEN: usize = 256;

/// A parsed friend code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendCode {
    pub public_key: PublicKeyHex,
    pub profile_key: RecordKeyStr,
    pub token: FriendRequestToken,
}

/// Why a friend code was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FriendCodeError {
    #[error("not a rekindle://add/ friend code")]
    NotAFriendCode,
    #[error("friend code is {got} bytes, max {max}")]
    TooLong { max: usize, got: usize },
    #[error("friend code needs a public key, a profile key and a token")]
    MissingPart,
    #[error("friend code has extra path segments")]
    ExtraPart,
    #[error("friend code public key: {0}")]
    PublicKey(KeyFormatError),
    #[error("friend code profile key: {0}")]
    ProfileKey(KeyFormatError),
    #[error("friend code token: {0}")]
    Token(KeyFormatError),
}

impl FriendCode {
    /// Parse a friend code. Surrounding whitespace is ignored; anything
    /// else that is not the exact canonical shape is rejected.
    pub fn parse(url: &str) -> Result<Self, FriendCodeError> {
        let url = url.trim();
        if url.len() > MAX_FRIEND_CODE_URL_LEN {
            return Err(FriendCodeError::TooLong {
                max: MAX_FRIEND_CODE_URL_LEN,
                got: url.len(),
            });
        }
        let rest = url
            .strip_prefix(FRIEND_CODE_PREFIX)
            .ok_or(FriendCodeError::NotAFriendCode)?;
        let mut parts = rest.split('/');
        let mut next = || {
            parts
                .next()
                .filter(|p| !p.is_empty())
                .ok_or(FriendCodeError::MissingPart)
        };
        let public_key = next()?;
        let profile_key = next()?;
        let token = next()?;
        if parts.next().is_some() {
            return Err(FriendCodeError::ExtraPart);
        }
        Ok(Self {
            public_key: key_format::public_key_hex(public_key)
                .map_err(FriendCodeError::PublicKey)?,
            profile_key: key_format::record_key(profile_key)
                .map_err(FriendCodeError::ProfileKey)?,
            token: key_format::friend_request_token(token).map_err(FriendCodeError::Token)?,
        })
    }

    /// The canonical code. [`FriendCode::parse`] of the result returns `self`.
    #[must_use]
    pub fn to_url(&self) -> String {
        format!(
            "{FRIEND_CODE_PREFIX}{}/{}/{}",
            self.public_key, self.profile_key, self.token
        )
    }
}

#[cfg(test)]
#[path = "friend_code_tests.rs"]
mod tests;

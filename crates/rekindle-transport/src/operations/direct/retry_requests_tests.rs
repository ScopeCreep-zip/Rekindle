use rekindle_crypto::CryptoError;

use super::*;

#[test]
fn only_user_content_under_a_session_is_requested() {
    let failed = RatchetError::Crypto(CryptoError::SessionError("aead".into()));
    assert!(requestable(DmTypeId::DmText, &failed));
    assert!(requestable(DmTypeId::DmReaction, &failed));
    // Not user content: nothing was logged to resend.
    assert!(!requestable(DmTypeId::Typing, &failed));
    assert!(!requestable(DmTypeId::RetryRequest, &failed));
    assert!(!requestable(DmTypeId::FriendsKey, &failed));
    // A copy of one already opened, or no session to carry a request.
    let dup = RatchetError::Crypto(CryptoError::DuplicateMessage);
    assert!(!requestable(DmTypeId::DmText, &dup));
    let none = RatchetError::Crypto(CryptoError::NoSession("peer".into()));
    assert!(!requestable(DmTypeId::DmText, &none));
}

#[test]
fn the_jitter_is_bounded() {
    assert!(RETRY_JITTER_MAX <= Duration::from_secs(5));
}

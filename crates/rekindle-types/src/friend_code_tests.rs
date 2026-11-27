use super::{FriendCode, FriendCodeError, MAX_FRIEND_CODE_URL_LEN};

const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const TOKEN: &str = rekindle_test_fixtures::FRIEND_TOKEN;

fn pk() -> String {
    "ab".repeat(32)
}

#[test]
fn the_longest_code_round_trips_under_its_cap() {
    let url = format!("rekindle://add/{}/VLD0:{B64}:{B64}/{TOKEN}", pk());
    let code = FriendCode::parse(&format!(" {url}\n")).unwrap();
    assert_eq!(code.to_url(), url);
    assert!(url.len() <= MAX_FRIEND_CODE_URL_LEN);
}

#[test]
fn rejects_malformed_codes() {
    let good_key = format!("VLD0:{B64}");
    let cases = [
        ("rekindle://invite/x".to_owned(), "prefix"),
        (format!("rekindle://add/{}/{good_key}", pk()), "missing"),
        (
            format!("rekindle://add/{}/{good_key}/{TOKEN}/x", pk()),
            "extra",
        ),
        (format!("rekindle://add/AB/{good_key}/{TOKEN}"), "key"),
        (
            format!("rekindle://add/{}/{good_key}/{}", pk(), "A".repeat(26)),
            "token",
        ),
        // A nonzero padding bit: a second spelling of the same bytes.
        (
            format!(
                "rekindle://add/{}/{good_key}/abcdefghijklmnopqrstuvwx2r",
                pk()
            ),
            "pad",
        ),
        (format!("rekindle://add/{}", "a".repeat(300)), "long"),
    ];
    for (url, why) in cases {
        let err = FriendCode::parse(&url).unwrap_err();
        let ok = match why {
            "prefix" => err == FriendCodeError::NotAFriendCode,
            "missing" => err == FriendCodeError::MissingPart,
            "extra" => err == FriendCodeError::ExtraPart,
            "key" => matches!(err, FriendCodeError::PublicKey(_)),
            "token" | "pad" => matches!(err, FriendCodeError::Token(_)),
            _ => matches!(err, FriendCodeError::TooLong { .. }),
        };
        assert!(ok, "{why}: {err:?}");
    }
}

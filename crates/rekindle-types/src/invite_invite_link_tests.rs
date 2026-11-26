use super::{
    DeepLink, DeepLinkError, InviteLink, InviteLinkError, MAX_FRIEND_INVITE_URL_LEN,
    MAX_INVITE_URL_LEN,
};

const B64: &str = "um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const CODE: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c";

const OP: &str = "5e0f9c2b7a1d3e4f6a8b0c2d4e6f8a0b1c3d5e7f9a2b4c6d8e0f1a3b5c7d9e2f";

fn link(gov: &str, sec: &str, code: &str) -> String {
    format!("rekindle://invite/{gov}/{sec}/{code}/{OP}/7/3")
}

#[test]
fn parses_and_round_trips_a_canonical_link() {
    let gov = format!("VLD0:{B64}:{B64}");
    let sec = format!("VLD0:{B64}");
    let url = link(&gov, &sec, CODE);
    let parsed = InviteLink::parse(&format!("  {url}\n")).unwrap();
    assert_eq!(parsed.anchor.as_str(), gov);
    assert_eq!(parsed.secrets_record_key.as_str(), sec);
    assert_eq!(parsed.invite_code.as_str(), CODE);
    assert_eq!(hex::encode(parsed.invite_op.0), OP);
    assert_eq!((parsed.invite_op_seq, parsed.frontier_seq), (7, 3));
    assert_eq!(parsed.to_url(), url);
    // The longest canonical link: both seqs at their widest.
    let widest = format!(
        "rekindle://invite/{gov}/{gov}/{CODE}/{OP}/{}/{}",
        u64::MAX,
        u32::MAX
    );
    assert_eq!(widest.len(), 333);
    assert_eq!(InviteLink::parse(&widest).unwrap().to_url(), widest);
    assert!(widest.len() <= MAX_INVITE_URL_LEN);
}

#[test]
fn rejects_everything_else() {
    let k = format!("VLD0:{B64}");
    let cases = [
        (
            "https://example.com".to_owned(),
            InviteLinkError::NotAnInvite,
        ),
        (
            format!("rekindle://community/{k}/{k}/{CODE}"),
            InviteLinkError::NotAnInvite,
        ),
        (
            format!("rekindle://invite/{k}/{k}"),
            InviteLinkError::MissingPart,
        ),
        (
            format!("rekindle://invite/{k}//{CODE}"),
            InviteLinkError::MissingPart,
        ),
        (
            format!("{}/", link(&k, &k, CODE)),
            InviteLinkError::ExtraPart,
        ),
        (
            format!("rekindle://invite/{k}/{k}/{CODE}/{OP}/7"),
            InviteLinkError::MissingPart,
        ),
        (
            link(&k, &k, &format!("{CODE}/x")),
            InviteLinkError::ExtraPart,
        ),
        (
            format!("rekindle://invite/{k}/{k}/{CODE}/{}/7/3", OP.to_uppercase()),
            InviteLinkError::InviteOp,
        ),
        (
            format!("rekindle://invite/{k}/{k}/{CODE}/{OP}/07/3"),
            InviteLinkError::Seq("op"),
        ),
        (
            format!("rekindle://invite/{k}/{k}/{CODE}/{OP}/7/+3"),
            InviteLinkError::Seq("frontier"),
        ),
        (
            format!(
                "rekindle://invite/{k}/{k}/{CODE}/{OP}/7/{}",
                u64::from(u32::MAX) + 1
            ),
            InviteLinkError::Seq("frontier"),
        ),
    ];
    for (url, err) in cases {
        assert_eq!(InviteLink::parse(&url), Err(err), "{url}");
    }
    assert!(matches!(
        InviteLink::parse(&link(&format!("VLD1:{B64}"), &k, CODE)),
        Err(InviteLinkError::Anchor(_))
    ));
    assert!(matches!(
        InviteLink::parse(&link(&k, "VLD0:short", CODE)),
        Err(InviteLinkError::SecretsRecordKey(_))
    ));
    assert!(matches!(
        InviteLink::parse(&link(&k, &k, &format!("{CODE}00"))),
        Err(InviteLinkError::InviteCode(_))
    ));
    assert!(matches!(
        InviteLink::parse(&link(&k, &k, &CODE.to_uppercase())),
        Err(InviteLinkError::InviteCode(_))
    ));
    assert!(matches!(
        InviteLink::parse(&format!("rekindle://invite/{}", "A".repeat(400))),
        Err(InviteLinkError::TooLong { .. })
    ));
}

#[test]
fn classifies_deep_links() {
    let k = format!("VLD0:{B64}");
    let invite = link(&k, &k, CODE);
    assert!(matches!(
        DeepLink::parse(&invite),
        Ok(DeepLink::CommunityInvite(_))
    ));
    assert!(matches!(
        DeepLink::parse("rekindle://invite/bad"),
        Err(DeepLinkError::CommunityInvite(_))
    ));
    assert_eq!(
        DeepLink::parse("rekindle://pair?code=1"),
        Ok(DeepLink::Pairing)
    );
    assert_eq!(
        DeepLink::parse(" rekindle://eyJhIjoxfQ "),
        Ok(DeepLink::FriendInvite("rekindle://eyJhIjoxfQ".into()))
    );
    assert_eq!(
        DeepLink::parse("https://x"),
        Err(DeepLinkError::NotRekindle)
    );
    for other in [
        "rekindle://",
        "rekindle://settings/x",
        "rekindle://a?b",
        "rekindle://a.b",
    ] {
        assert_eq!(
            DeepLink::parse(other),
            Err(DeepLinkError::Unknown),
            "{other}"
        );
    }
    let code = format!(
        "rekindle://add/{}/VLD0:{B64}/abcdefghijklmnopqrstuvwx2q",
        "ab".repeat(32)
    );
    match DeepLink::parse(&code) {
        Ok(DeepLink::Add(parsed)) => assert_eq!(parsed.to_url(), code),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        DeepLink::parse("rekindle://add/x"),
        Err(DeepLinkError::FriendCode(_))
    ));
    let at_cap = format!("rekindle://{}", "A".repeat(MAX_FRIEND_INVITE_URL_LEN - 11));
    assert_eq!(at_cap.len(), MAX_FRIEND_INVITE_URL_LEN);
    assert!(matches!(
        DeepLink::parse(&at_cap),
        Ok(DeepLink::FriendInvite(_))
    ));
    assert!(matches!(
        DeepLink::parse(&format!("{at_cap}A")),
        Err(DeepLinkError::TooLong { .. })
    ));
}

/// The hostile corpus the webview spec used to probe. Every entry is
/// rejected before anything is held for consent.
#[test]
fn hostile_invite_links_are_rejected() {
    let corpus = [
        "rekindle://invite/".to_owned(),
        "rekindle://invite/a".to_owned(),
        "rekindle://invite/!!!!#@@@@".to_owned(),
        "rekindle://invite/QUFBQUFBQUFBQQ==#Wm16YnRtbk9hQQ==".to_owned(),
        "rekindle://invite/<script>alert(1)</script>#key".to_owned(),
        "rekindle://invite/../../../etc/passwd#k".to_owned(),
        "rekindle://invite/blob';DROP TABLE friends;--#k".to_owned(),
        format!(
            "rekindle://invite/{}#{}",
            "A".repeat(100_000),
            "B".repeat(10_000)
        ),
        "rekindle://invite/aaa%00bbb#%00".to_owned(),
        "rekindle://invite/aaa\u{0}bbb/x/y".to_owned(),
        "rekindle://invite/aaa%01%02%03#%04".to_owned(),
        "rekindle://invite/blob\u{202E}#key".to_owned(),
        format!("rekindle://invite/VLD0:{B64}/VLD0:{B64}/{CODE}#frag"),
        format!("rekindle://invite/VLD0:{B64}/VLD0:{B64}/{CODE}?q=1"),
    ];
    for url in &corpus {
        assert!(InviteLink::parse(url).is_err(), "{url:.60}");
        assert!(DeepLink::parse(url).is_err(), "{url:.60}");
    }
}

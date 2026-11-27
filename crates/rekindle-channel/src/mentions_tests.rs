use super::*;

#[test]
fn matches_default_is_empty_and_not_any() {
    let m = MentionMatches::default();
    assert!(!m.is_any());
    assert!(!m.everyone);
    assert!(!m.here);
    assert!(m.roles.is_empty());
    assert!(m.members.is_empty());
}

#[test]
fn matches_is_any_each_branch() {
    for m in [
        MentionMatches {
            everyone: true,
            ..Default::default()
        },
        MentionMatches {
            here: true,
            ..Default::default()
        },
        MentionMatches {
            roles: vec!["mod".into()],
            ..Default::default()
        },
        MentionMatches {
            members: vec!["alice".into()],
            ..Default::default()
        },
    ] {
        assert!(m.is_any());
    }
}

#[test]
fn scan_basic_tokens() {
    let tokens = scan_raw_tokens("hello @everyone and @alice and @here");
    assert_eq!(tokens, vec!["everyone", "alice", "here"]);
}

#[test]
fn scan_ignores_email_address() {
    let tokens = scan_raw_tokens("send to foo@bar.com please");
    assert!(tokens.is_empty());
}

#[test]
fn scan_punctuation_boundary() {
    let tokens = scan_raw_tokens("hey,@here!");
    assert_eq!(tokens, vec!["here"]);
}

#[test]
fn scan_hyphenated_token() {
    let tokens = scan_raw_tokens("ping @event-staff and @alice-jones");
    assert_eq!(tokens, vec!["event-staff", "alice-jones"]);
}

#[test]
fn scan_underscore_token() {
    let tokens = scan_raw_tokens("@my_role hi");
    assert_eq!(tokens, vec!["my_role"]);
}

#[test]
fn scan_start_of_string_token() {
    let tokens = scan_raw_tokens("@start mention");
    assert_eq!(tokens, vec!["start"]);
}

#[test]
fn scan_double_at_skips() {
    // @@ has an alphanumeric requirement after the inner @ — the
    // first @ has no token; the second @ is preceded by punctuation
    // ('@' is ascii_punctuation), so it does trigger.
    let tokens = scan_raw_tokens("@@hello");
    assert_eq!(tokens, vec!["hello"]);
}

#[test]
fn scan_empty_token_skipped() {
    let tokens = scan_raw_tokens("@ @  @");
    assert!(tokens.is_empty());
}

#[test]
fn has_perm_matches_bitflag_semantics() {
    const A: u64 = 0b0001;
    const B: u64 = 0b0010;
    const C: u64 = 0b0100;
    assert!(has_perm(A | B, A));
    assert!(has_perm(A | B, B));
    assert!(has_perm(A | B | C, A | B));
    assert!(!has_perm(A, B));
    assert!(!has_perm(A | C, A | B));
    assert!(has_perm(u64::MAX, A | B | C));
}

#[test]
fn mass_mentions_escalate_only_past_the_online_count() {
    use rekindle_types::intent::Escalation;
    let everyone = MentionMatches {
        everyone: true,
        ..MentionMatches::default()
    };
    assert_eq!(
        mass_mention_escalation(&everyone, 214, 12, 0),
        Some(Escalation::MassMention { notified: 214 })
    );
    assert_eq!(mass_mention_escalation(&everyone, 10, 10, 0), None);
    let here = MentionMatches {
        here: true,
        ..MentionMatches::default()
    };
    assert_eq!(mass_mention_escalation(&here, 214, 12, 0), None);
    let role = MentionMatches {
        roles: vec!["raiders".into()],
        ..MentionMatches::default()
    };
    assert_eq!(
        mass_mention_escalation(&role, 214, 12, 40),
        Some(Escalation::MassMention { notified: 40 })
    );
    assert_eq!(
        mass_mention_escalation(&MentionMatches::default(), 214, 12, 0),
        None
    );
}

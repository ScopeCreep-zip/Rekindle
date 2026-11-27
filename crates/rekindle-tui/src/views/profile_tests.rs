use super::*;

fn total(name: &str, secs: u64) -> GameTotalView {
    GameTotalView {
        game_id: 4181,
        game_name: name.into(),
        icon_key: None,
        total_secs: secs,
        last_played: now_ms() - 120_000,
    }
}

fn text(view: &ProfileView) -> Vec<String> {
    view.lines().iter().map(ToString::to_string).collect()
}

#[test]
fn a_friends_profile_shows_their_bio_and_shared_totals() {
    let key = "a".repeat(64);
    let mut view = ProfileView::new(Some(key.clone()), "alice".into());
    view.on_command_result(CommandResult::ProfileLoaded {
        peer_key: Some(key),
        bio: Some("Sniper main.\u{1b}[31m".into()),
        totals: vec![total("Counter-Strike 2", 9000)],
    })
    .unwrap();
    let lines = text(&view);
    assert!(lines[1].contains("Sniper main."));
    assert!(!lines[1].contains('\u{1b}'), "the bio is sanitized");
    let row = lines
        .iter()
        .find(|l| l.contains("Counter-Strike 2"))
        .unwrap();
    assert!(row.contains("2h 30m") && row.contains("last played 2m ago"));
}

#[test]
fn another_profiles_answer_is_ignored_and_none_shared_says_so() {
    let mut view = ProfileView::new(Some("a".repeat(64)), "alice".into());
    view.on_command_result(CommandResult::ProfileLoaded {
        peer_key: None,
        bio: Some("mine".into()),
        totals: vec![],
    })
    .unwrap();
    assert_eq!(text(&view), ["Loading…"]);
    view.on_command_result(CommandResult::ProfileLoaded {
        peer_key: Some("a".repeat(64)),
        bio: None,
        totals: vec![],
    })
    .unwrap();
    let lines = text(&view);
    assert!(lines.contains(&"  No bio".to_string()));
    assert!(lines.contains(&"  Nothing shared".to_string()));
}

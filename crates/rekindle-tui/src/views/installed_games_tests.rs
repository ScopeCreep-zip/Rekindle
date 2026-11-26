use super::*;

fn game(game_id: u32, name: &str, launchable: bool) -> InstalledGameView {
    InstalledGameView {
        game_id,
        game_name: name.into(),
        icon_key: None,
        launchable,
    }
}

fn loaded() -> InstalledGamesView {
    let mut view = InstalledGamesView::new();
    view.on_command_result(CommandResult::InstalledGamesLoaded {
        games: vec![
            game(4181, "Counter-Strike 2", true),
            game(8507, "Battlefield 2", false),
        ],
    })
    .unwrap();
    view
}

#[test]
fn enter_launches_the_selected_installed_game_without_a_server() {
    let mut view = loaded();
    let Ok(Some(Action::LaunchGame {
        game_id,
        server,
        source,
    })) = view.update(Action::Select)
    else {
        panic!("Enter launches a launchable game");
    };
    assert_eq!(
        (game_id, server, source),
        (4181, None, JoinSource::Installed)
    );
}

#[test]
fn a_game_that_cannot_launch_is_listed_and_says_why() {
    let mut view = loaded();
    view.update(Action::ScrollDown(1)).unwrap();
    assert_eq!(
        row(&view.games[1]),
        ("Battlefield 2".into(), "  (can't be launched here)".into())
    );
    let Ok(Some(Action::ShowToast { message, .. })) = view.update(Action::Select) else {
        panic!("a disabled launch shows the reason and sends nothing");
    };
    assert_eq!(message, "Battlefield 2 can't be launched here");
}

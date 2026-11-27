use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn server(address: &str, invitable: bool) -> FavouriteServerView {
    FavouriteServerView {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        icon_key: None,
        address: address.into(),
        label: "scrims".into(),
        added_at: 0,
        invitable,
    }
}

#[test]
fn x_removes_the_selected_server_and_rows_name_uninvitable_ones() {
    let mut view = FavouritesView::new();
    view.on_command_result(CommandResult::FavouritesLoaded {
        servers: vec![server("10.0.0.1:27015", true), server("[::1]:27016", false)],
    })
    .unwrap();
    view.handle_focused_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let Some(Action::RemoveFavourite { game_id, server }) =
        view.handle_focused_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
    else {
        panic!("x asks to remove the selected server");
    };
    assert_eq!((game_id, server.as_str()), (4181, "[::1]:27016"));
    assert_eq!(
        row(&view.servers[0]),
        (
            "Counter-Strike 2  10.0.0.1:27015".into(),
            "  \"scrims\"".into()
        )
    );
    assert_eq!(row(&view.servers[1]).1, "  \"scrims\"  (no invites)");
}

#[test]
fn enter_launches_the_selected_favourite_to_its_server() {
    let mut view = FavouritesView::new();
    view.on_command_result(CommandResult::FavouritesLoaded {
        servers: vec![server("10.0.0.1:27015", true), server("[::1]:27016", false)],
    })
    .unwrap();
    view.update(Action::ScrollDown(1)).unwrap();
    let Ok(Some(Action::LaunchGame {
        game_id,
        server,
        source,
    })) = view.update(Action::Select)
    else {
        panic!("Enter launches the selected favourite");
    };
    assert_eq!(
        (game_id, server.as_ref().map(ServerAddress::as_str), source),
        (4181, Some("[::1]:27016"), JoinSource::Favourite)
    );
}

#[test]
fn the_input_line_adds_a_parsed_server_or_says_why_not() {
    let Ok(Action::AddFavourite {
        game_id,
        server,
        label,
    }) = FavouritesView::parse_add("4181 10.0.0.1:27015 EU scrims")
    else {
        panic!("a game id, an address and a label add a server");
    };
    assert_eq!(
        (game_id, server.as_str(), label.as_str()),
        (4181, "10.0.0.1:27015", "EU scrims")
    );
    assert!(FavouritesView::parse_add("4181").is_err());
    assert!(FavouritesView::parse_add("cs2 10.0.0.1:27015").is_err());
    assert!(FavouritesView::parse_add("4181 10.0.0.1:27015;rm").is_err());
}

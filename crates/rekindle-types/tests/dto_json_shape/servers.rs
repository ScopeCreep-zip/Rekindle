//! The Servers view (plan E7.2 G3) in the golden DTO file: every category
//! kind, every row source, both verdict and Join shapes.

use rekindle_types::dto::servers::{
    Joinability, ServerCategory, ServerCategoryKind, ServerCommunity, ServerFriend, ServerJoin,
    ServerRefusal, ServerRow, ServerVerdict, ServersView,
};
use rekindle_types::friend::NotJoinableReason;
use rekindle_types::game_server::JoinSource;
use serde_json::Value;

fn row(game_id: Option<u32>, source: JoinSource, joinability: Joinability) -> ServerRow {
    ServerRow {
        game_id,
        game_name: "Counter-Strike 2".into(),
        icon_key: game_id.map(|_| "cs2".into()),
        address: "203.0.113.7:27015".into(),
        label: String::new(),
        source,
        friend: None,
        last_seen: None,
        join: ServerJoin {
            game_id,
            server: Some("203.0.113.7:27015".into()),
            launch_source: source,
            joinability,
        },
        add_favourite: ServerVerdict::Allowed,
        remove: ServerVerdict::refused(ServerRefusal::NotRemovable),
    }
}

fn category(kind: ServerCategoryKind, rows: Vec<ServerRow>) -> ServerCategory {
    ServerCategory {
        kind,
        community: None,
        rows,
    }
}

/// The whole view: every category kind and row source.
fn view(pk: &str) -> ServersView {
    let cannot = Joinability::NotJoinable {
        reason: NotJoinableReason::CannotLaunch,
    };
    let favourite = ServerRow {
        label: "Scrims".into(),
        add_favourite: ServerVerdict::refused(ServerRefusal::AlreadyFavourite),
        remove: ServerVerdict::Allowed,
        ..row(Some(4_181), JoinSource::Favourite, cannot)
    };
    let recent = ServerRow {
        last_seen: Some(1_700_000_000),
        remove: ServerVerdict::Allowed,
        ..row(Some(4_181), JoinSource::Recent, Joinability::Joinable)
    };
    let friend = ServerRow {
        friend: Some(ServerFriend {
            public_key: pk.to_owned(),
            display_name: "Bob".into(),
        }),
        ..row(
            Some(4_181),
            JoinSource::FriendPresence,
            Joinability::Joinable,
        )
    };
    let invite = ServerRow {
        friend: friend.friend.clone(),
        ..row(Some(4_181), JoinSource::Invite, Joinability::Joinable)
    };
    let community_row = ServerRow {
        game_name: "minecraft".into(),
        label: "Survival".into(),
        add_favourite: ServerVerdict::refused(ServerRefusal::NoNumericGame),
        ..row(
            None,
            JoinSource::Community,
            Joinability::NotJoinable {
                reason: NotJoinableReason::CannotLaunch,
            },
        )
    };
    let mut community = category(ServerCategoryKind::Community, vec![community_row]);
    community.community = Some(ServerCommunity {
        anchor: "VLD0:um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2".into(),
        name: "Frag Night".into(),
    });
    ServersView {
        categories: vec![
            category(ServerCategoryKind::Favourites, vec![favourite]),
            category(ServerCategoryKind::Recent, vec![recent]),
            category(ServerCategoryKind::Friends, vec![friend, invite]),
            community,
        ],
    }
}

pub fn samples(add: &mut impl FnMut(&str, Value), pk: &str) {
    add("serversView", serde_json::to_value(view(pk)).unwrap());
    add(
        "serversViewEmpty",
        serde_json::to_value(ServersView {
            categories: vec![category(ServerCategoryKind::Recent, vec![])],
        })
        .unwrap(),
    );
}

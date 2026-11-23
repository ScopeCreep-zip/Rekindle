//! Local search (plan E7.2 G11, SK2-G §8) in the golden DTO file: every
//! `LocalSearchScope`, every `LocalHit` kind, the answer, and the
//! `ShellOp::OpenSearch` a skin's `SearchFor` runs.

use rekindle_types::dto::local_search::{LocalHit, LocalSearchResult, LocalSearchScope};
use rekindle_types::dto::skin_run::ShellOp;
use rekindle_types::dto::transfer::TransferTarget;
use rekindle_types::search::{SearchHit, SearchScope};
use serde_json::Value;

fn to(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serialize sample")
}

fn hits(pk: &str) -> Vec<LocalHit> {
    vec![
        LocalHit::Friend {
            public_key: pk.to_owned(),
            name: "Alice".into(),
        },
        LocalHit::GroupDm {
            group_id: "0a".repeat(16),
            name: "Raid night".into(),
        },
        LocalHit::Member {
            community: "VLD0:community".into(),
            pseudonym: pk.to_owned(),
            name: "Bob".into(),
        },
        LocalHit::Server {
            game_id: 4100,
            address: "203.0.113.7:27015".into(),
            label: "Clan server".into(),
        },
        LocalHit::File {
            attachment_id: "att-1".into(),
            target: TransferTarget::Channel {
                community: "VLD0:community".into(),
                channel_id: "ch".into(),
            },
            name: "map.zip".into(),
        },
        LocalHit::Message(SearchHit {
            scope: SearchScope::Dm,
            conversation_id: pk.to_owned(),
            message_id: Some("m1".into()),
            sender_key: pk.to_owned(),
            body: "see you at nine".into(),
            timestamp: 1_700_000_000_000,
            rank: 1.5,
            before_body: None,
            after_body: Some("ok".into()),
        }),
    ]
}

pub(super) fn samples(add: &mut impl FnMut(&str, Value), pk: &str) {
    let scopes = [
        ("friends", LocalSearchScope::Friends),
        ("servers", LocalSearchScope::Servers),
        ("files", LocalSearchScope::Files),
        (
            "conversation",
            LocalSearchScope::Conversation {
                conversation_id: pk.to_owned(),
            },
        ),
    ];
    for (name, scope) in scopes {
        add(&format!("localSearch.scope.{name}"), to(&scope));
    }
    let names = ["friend", "groupDm", "member", "server", "file", "message"];
    for (name, hit) in names.iter().zip(hits(pk)) {
        add(&format!("localSearch.hit.{name}"), to(hit));
    }
    add(
        "localSearch.result",
        to(LocalSearchResult {
            hits: hits(pk).into_iter().take(1).collect(),
        }),
    );
    add(
        "skin.shell.openSearch",
        to(ShellOp::OpenSearch {
            scope: LocalSearchScope::Files,
        }),
    );
}

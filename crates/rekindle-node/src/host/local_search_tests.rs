//! `LocalSearch` through the Host (plan E7.2 G11, SK2-G §8): answered from
//! the identity's database, in the contract's JSON shape; a malformed
//! conversation id is refused before anything is read.

use rekindle_db::repo::favourite_servers::{self, FavouriteServer};
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::dto::local_search::LocalSearchScope;

use super::direct_tests::ann;

#[tokio::test]
async fn the_host_answers_a_local_search_from_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let owner = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| {
            favourite_servers::add(
                c,
                &owner,
                &FavouriteServer {
                    game_id: 4100,
                    address: "203.0.113.7:27015".into(),
                    label: "Clan server".into(),
                    added_at: 1,
                },
            )
        })
        .await
        .unwrap();
    let search = |scope, query: &str| IpcRequest::LocalSearch {
        scope,
        query: query.into(),
    };
    let answer = host
        .call(Caller::shell(), search(LocalSearchScope::Servers, "clan"))
        .await;
    let IpcResponse::Ok { data, .. } = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        data,
        serde_json::json!({ "hits": [{
            "kind": "server",
            "arg": { "gameId": 4100, "address": "203.0.113.7:27015", "label": "Clan server" },
        }] })
    );
    // The same query in the buddy list's scope finds no server.
    let answer = host
        .call(Caller::shell(), search(LocalSearchScope::Friends, "clan"))
        .await;
    assert!(
        matches!(&answer, IpcResponse::Ok { data, .. } if data["hits"] == serde_json::json!([])),
        "{answer:?}"
    );
    let refused = host
        .call(
            Caller::shell(),
            search(
                LocalSearchScope::Conversation {
                    conversation_id: "not a key".into(),
                },
                "hi",
            ),
        )
        .await;
    assert!(
        matches!(refused, IpcResponse::Error { code: 400, .. }),
        "{refused:?}"
    );
}

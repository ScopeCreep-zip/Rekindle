use super::*;
use crate::repo::communities::{self, NewCommunity};
use crate::repo::fixture::{self, OWNER};

fn channel(id: &str) -> StoredChannel<'_> {
    StoredChannel {
        id,
        name: "general",
        channel_type: "text",
        category_id: None,
        topic: "",
        slowmode_seconds: 0,
        nsfw: false,
        mek_generation: 1,
        parent_voice_channel_id: None,
    }
}

#[test]
fn a_channel_is_inserted_once_and_deleted() {
    let conn = fixture::conn();
    communities::insert(
        &conn,
        OWNER,
        &NewCommunity {
            id: "c1",
            name: "C",
            joined_at: 1,
            my_pseudonym_key: "p",
            mek_generation: 1,
            member_registry_key: None,
            my_subkey_index: None,
        },
    )
    .unwrap();
    insert(&conn, OWNER, "c1", &channel("a")).unwrap();
    insert_if_absent(&conn, OWNER, "c1", &channel("a")).unwrap();
    assert!(insert(&conn, OWNER, "c1", &channel("a")).is_err());
    delete(&conn, OWNER, "c1", "a").unwrap();
    insert(&conn, OWNER, "c1", &channel("a")).unwrap();
}

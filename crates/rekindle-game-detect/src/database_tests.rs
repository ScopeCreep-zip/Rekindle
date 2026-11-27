use super::*;

#[test]
fn load_and_lookup() {
    let json = r#"{
            "games": [
                {
                    "id": 4181,
                    "name": "Counter-Strike 2",
                    "process_names": ["cs2.exe", "cs2"],
                    "icon": "cs2"
                }
            ]
        }"#;

    let db = GameDatabase::from_json(json).unwrap();
    assert_eq!(db.game_count(), 1);

    let entry = db.lookup_by_process("cs2.exe").unwrap();
    assert_eq!(entry.name, "Counter-Strike 2");

    // Case insensitive
    let entry = db.lookup_by_process("CS2.EXE").unwrap();
    assert_eq!(entry.id, 4181);
}

/// Counter-Strike 2 is in Xfire's list (`xfire_ids.json`), so its
/// icon is Xfire's; Valorant is not, so it keeps its slug; an unknown
/// game has none.
#[test]
fn icon_keys_prefer_the_xfire_id_then_the_slug() {
    assert_eq!(
        GameDatabase::icon_key_for(4181).as_deref(),
        Some("xfire/8507")
    );
    let unmapped = GameDatabase::shared()
        .by_process
        .values()
        .find(|e| e.xfire_id.is_none())
        .expect("a game Xfire never listed");
    assert_eq!(
        unmapped.icon_key(),
        Some(format!("slug/{}", unmapped.icon.as_ref().unwrap()))
    );
    assert_eq!(GameDatabase::icon_key_for(1), None);
}

/// A launcher substitutes the address once: a template or network
/// arguments with no `{addr}`, or two, would launch somewhere the user
/// did not pick.
#[test]
fn every_connect_template_holds_one_address() {
    let db = GameDatabase::bundled();
    for entry in db.by_process.values() {
        for template in [&entry.connect_template, &entry.network_args]
            .into_iter()
            .flatten()
        {
            assert_eq!(
                template.matches("{addr}").count(),
                1,
                "{}: {template}",
                entry.name
            );
        }
    }
}

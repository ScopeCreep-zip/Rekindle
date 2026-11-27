use super::*;

#[test]
fn an_archive_round_trips_and_damage_is_refused() {
    let archive = Archive {
        entry: IdentityEntry {
            public_key: "pk".into(),
            display_name: "Name".into(),
            created_at: 7,
        },
        header: vec![1; 64],
        vault: vec![2; 300],
    };
    let bytes = archive.encode().unwrap();
    let back = Archive::decode(&bytes).unwrap();
    assert_eq!(back.entry, archive.entry);
    assert_eq!(back.header, archive.header);
    assert_eq!(back.vault, archive.vault);
    assert!(Archive::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(Archive::decode(&longer).is_err());
    assert!(Archive::decode(b"nope").is_err());
}

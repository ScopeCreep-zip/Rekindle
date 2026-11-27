use super::pick;

#[test]
fn names_list_every_match_and_exact_ids_take_precedence() {
    let communities = [("a1", "Dev"), ("a2", "DEV"), ("a3", "Dev"), ("a4", "a1")];
    let find = |given: &str| {
        pick(communities.into_iter(), given, "community", |name| {
            name.eq_ignore_ascii_case(given)
        })
    };

    assert_eq!(find("a1").unwrap(), "a1");
    assert_eq!(
        find("dev").unwrap_err().to_string(),
        "ambiguous community \"dev\": Dev (a1), DEV (a2), Dev (a3); give the ID"
    );

    let channels = [("c1", "general"), ("c2", "general")];
    let error = pick(channels.into_iter(), "general", "channel", |name| {
        name == "general"
    })
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "ambiguous channel \"general\": general (c1), general (c2); give the ID"
    );
}

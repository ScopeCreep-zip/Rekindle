use super::*;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::MemberPresence;

fn row(pk: u8, display: Option<&str>, bio: Option<&str>) -> DiscoveredRow {
    let mut bytes = [0u8; 32];
    bytes[0] = pk;
    let presence = MemberPresence {
        pseudonym_key: PseudonymKey(bytes),
        display_name: display.map(str::to_string),
        bio: bio.map(str::to_string),
        ..Default::default()
    };
    (0u32, 0u32, presence)
}

fn snapshot(display: Option<&str>, bio: Option<&str>) -> MemberProfileSnapshot {
    MemberProfileSnapshot {
        display_name: display.map(str::to_string),
        bio: bio.map(str::to_string),
        ..Default::default()
    }
}

#[test]
fn empty_discovered_yields_no_changes() {
    let prior: HashMap<String, MemberProfileSnapshot> = HashMap::new();
    let outcome = compute_profile_diff(&prior, &[]);
    assert!(!outcome.changed);
    assert!(outcome.updates.is_empty());
}

#[test]
fn unchanged_row_is_not_flagged() {
    let mut bytes = [0u8; 32];
    bytes[0] = 1;
    let pk_hex = hex::encode(bytes);
    let mut prior = HashMap::new();
    prior.insert(pk_hex.clone(), snapshot(Some("alice"), Some("hi")));
    let outcome = compute_profile_diff(&prior, &[row(1, Some("alice"), Some("hi"))]);
    assert!(!outcome.changed);
    assert!(outcome.updates.is_empty());
}

#[test]
fn changed_display_name_flagged() {
    let mut bytes = [0u8; 32];
    bytes[0] = 2;
    let pk_hex = hex::encode(bytes);
    let mut prior = HashMap::new();
    prior.insert(pk_hex.clone(), snapshot(Some("alice"), Some("hi")));
    let outcome = compute_profile_diff(&prior, &[row(2, Some("alice-renamed"), Some("hi"))]);
    assert!(outcome.changed);
    assert_eq!(outcome.updates.len(), 1);
    assert_eq!(
        outcome.updates.get(&pk_hex).unwrap().display_name,
        Some("alice-renamed".to_string())
    );
}

#[test]
fn new_row_is_flagged_and_added_to_updates() {
    let prior: HashMap<String, MemberProfileSnapshot> = HashMap::new();
    let outcome = compute_profile_diff(&prior, &[row(3, Some("bob"), None)]);
    assert!(outcome.changed);
    assert_eq!(outcome.updates.len(), 1);
}

#[test]
fn missing_field_added_in_new_snapshot_is_flagged() {
    let mut bytes = [0u8; 32];
    bytes[0] = 4;
    let pk_hex = hex::encode(bytes);
    let mut prior = HashMap::new();
    prior.insert(pk_hex.clone(), snapshot(Some("alice"), None));
    let outcome = compute_profile_diff(&prior, &[row(4, Some("alice"), Some("now has bio"))]);
    assert!(outcome.changed);
}

#[test]
fn changed_location_flagged() {
    use rekindle_types::presence::SessionLocation;
    let mut bytes = [0u8; 32];
    bytes[0] = 7;
    let pk_hex = hex::encode(bytes);
    let mut prior = HashMap::new();
    prior.insert(pk_hex.clone(), snapshot(Some("carol"), None));

    // Build a row whose session has a text location.
    let mut row = row(7, Some("carol"), None);
    row.2.session.location = Some(SessionLocation::Text {
        channel_id: "abc".to_string(),
    });

    let outcome = compute_profile_diff(&prior, &[row]);
    assert!(outcome.changed, "a new location must flag a refresh");
    assert_eq!(
        outcome.updates.get(&pk_hex).unwrap().location,
        Some(SessionLocation::Text {
            channel_id: "abc".to_string(),
        })
    );
}

#[test]
fn multiple_rows_only_changed_one_yields_single_update() {
    let mut bytes1 = [0u8; 32];
    bytes1[0] = 5;
    let pk1 = hex::encode(bytes1);
    let mut bytes2 = [0u8; 32];
    bytes2[0] = 6;
    let pk2 = hex::encode(bytes2);
    let mut prior = HashMap::new();
    prior.insert(pk1.clone(), snapshot(Some("alice"), None));
    prior.insert(pk2.clone(), snapshot(Some("bob"), None));
    let outcome = compute_profile_diff(
        &prior,
        &[
            row(5, Some("alice"), None),       // unchanged
            row(6, Some("bob-renamed"), None), // changed
        ],
    );
    assert!(outcome.changed);
    assert_eq!(outcome.updates.len(), 1);
    assert!(outcome.updates.contains_key(&pk2));
}

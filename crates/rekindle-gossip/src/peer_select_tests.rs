use super::*;

fn peer(name: &str) -> PeerInfo {
    PeerInfo {
        pseudonym_key: name.to_string(),
        route_blob: vec![0u8],
    }
}

#[test]
fn sorts_descending_by_score() {
    let peers = vec![peer("alice"), peer("bob"), peer("carol")];
    let mut scores = HashMap::new();
    scores.insert("alice".to_string(), 0.3);
    scores.insert("bob".to_string(), 0.9);
    scores.insert("carol".to_string(), 0.5);
    let ordered = sort_peers_by_reliability(peers, &scores);
    assert_eq!(
        ordered
            .iter()
            .map(|p| p.pseudonym_key.as_str())
            .collect::<Vec<_>>(),
        vec!["bob", "carol", "alice"]
    );
}

#[test]
fn unknown_peers_get_neutral_score() {
    let peers = vec![peer("known"), peer("unknown")];
    let mut scores = HashMap::new();
    scores.insert("known".to_string(), 0.7);
    let ordered = sort_peers_by_reliability(peers, &scores);
    // known (0.7) ranked above unknown (0.5 default)
    assert_eq!(ordered[0].pseudonym_key, "known");
    assert_eq!(ordered[1].pseudonym_key, "unknown");
}

#[test]
fn ties_broken_by_pseudonym_ascending() {
    let peers = vec![peer("charlie"), peer("alice"), peer("bob")];
    let scores = HashMap::new(); // all neutral
    let ordered = sort_peers_by_reliability(peers, &scores);
    assert_eq!(ordered[0].pseudonym_key, "alice");
    assert_eq!(ordered[1].pseudonym_key, "bob");
    assert_eq!(ordered[2].pseudonym_key, "charlie");
}

#[test]
fn empty_input_returns_empty() {
    let ordered = sort_peers_by_reliability(Vec::new(), &HashMap::new());
    assert!(ordered.is_empty());
}

#[test]
fn scores_from_counters_handles_zero_total() {
    let mut counters = HashMap::new();
    counters.insert("fresh".to_string(), (0u32, 0u32));
    counters.insert("active".to_string(), (8u32, 2u32));
    let scores = scores_from_counters(&counters);
    assert!((scores["fresh"] - 0.5).abs() < f64::EPSILON);
    assert!((scores["active"] - 0.8).abs() < f64::EPSILON);
}

#[test]
fn scores_from_counters_pure_failures_yields_zero() {
    let mut counters = HashMap::new();
    counters.insert("offline".to_string(), (0u32, 5u32));
    let scores = scores_from_counters(&counters);
    assert!((scores["offline"] - 0.0).abs() < f64::EPSILON);
}

use super::*;

fn graph(edges: &[(&str, &[&str])]) -> BTreeMap<String, BTreeSet<String>> {
    edges
        .iter()
        .map(|(from, to)| {
            (
                (*from).to_string(),
                to.iter().map(|t| (*t).to_string()).collect(),
            )
        })
        .collect()
}

fn members(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

#[test]
fn a_tier_crate_that_reaches_veilid_through_another_crate_fails() {
    // The planted dependency: rekindle-voice → rekindle-protocol → veilid-core.
    let g = graph(&[
        ("rekindle-voice", &["rekindle-codec", "rekindle-protocol"]),
        ("rekindle-protocol", &["veilid-core", "rekindle-codec"]),
        ("rekindle-codec", &["rekindle-types"]),
    ]);
    assert_eq!(
        violations(&g, &members(&["rekindle-voice", "rekindle-protocol"])),
        vec!["rekindle-voice".to_string()]
    );
}

#[test]
fn veilid_free_tier_crates_and_the_allowlist_pass() {
    let g = graph(&[
        ("rekindle-voice", &["rekindle-codec"]),
        ("rekindle-codec", &["rekindle-types"]),
        ("rekindle-protocol", &["veilid-core", "rekindle-codec"]),
        ("rekindle-node", &["rekindle-transport"]),
        ("rekindle-transport", &["rekindle-protocol"]),
    ]);
    assert!(violations(
        &g,
        &members(&[
            "rekindle-voice",
            "rekindle-codec",
            "rekindle-protocol",
            "rekindle-node"
        ])
    )
    .is_empty());
}

#[test]
fn a_cycle_in_the_graph_terminates() {
    let g = graph(&[("a", &["b"]), ("b", &["a", "veilid-core"])]);
    assert_eq!(violations(&g, &members(&["a"])), vec!["a".to_string()]);
}

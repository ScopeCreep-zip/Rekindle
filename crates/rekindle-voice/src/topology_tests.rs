use super::*;

fn mesh() -> VoiceMode {
    VoiceMode::Mesh
}

fn mcu(host: &str) -> VoiceMode {
    VoiceMode::Mcu {
        host_pseudonym: host.to_string(),
    }
}

#[test]
fn join_below_threshold_stays_mesh() {
    // peer_count = 3 → total = 4, still mesh per §10.2.
    assert_eq!(
        decide_mode_after_join(3, &mesh(), false, Some("alice")),
        ModeDecision::NoChange,
    );
}

#[test]
fn join_at_threshold_switches_to_mcu() {
    // peer_count = 4 → total = 5, switch to MCU.
    assert_eq!(
        decide_mode_after_join(4, &mesh(), false, Some("alice")),
        ModeDecision::SwitchToMcu {
            host: "alice".into()
        },
    );
}

#[test]
fn join_at_threshold_without_elected_host_no_change() {
    // Election failed — orchestrator handles the warn path.
    assert_eq!(
        decide_mode_after_join(4, &mesh(), false, None),
        ModeDecision::NoChange,
    );
}

#[test]
fn join_above_threshold_in_mcu_no_change() {
    assert_eq!(
        decide_mode_after_join(5, &mcu("alice"), false, Some("bob")),
        ModeDecision::NoChange,
    );
}

#[test]
fn stage_channel_join_never_switches() {
    assert_eq!(
        decide_mode_after_join(10, &mesh(), true, Some("alice")),
        ModeDecision::NoChange,
    );
}

#[test]
fn leave_mesh_no_change() {
    assert_eq!(
        decide_mode_after_leave(2, &mesh(), false, false, None),
        ModeDecision::NoChange,
    );
}

#[test]
fn leave_mcu_below_threshold_switches_to_mesh() {
    assert_eq!(
        decide_mode_after_leave(3, &mcu("alice"), false, false, None),
        ModeDecision::SwitchToMesh,
    );
}

#[test]
fn leave_mcu_host_left_with_reelection() {
    assert_eq!(
        decide_mode_after_leave(4, &mcu("alice"), false, true, Some("bob")),
        ModeDecision::SwitchToMcu { host: "bob".into() },
    );
}

#[test]
fn leave_mcu_host_left_below_threshold_to_mesh() {
    assert_eq!(
        decide_mode_after_leave(3, &mcu("alice"), false, true, Some("bob")),
        ModeDecision::SwitchToMesh,
    );
}

#[test]
fn leave_mcu_host_left_no_election_to_mesh() {
    assert_eq!(
        decide_mode_after_leave(4, &mcu("alice"), false, true, None),
        ModeDecision::SwitchToMesh,
    );
}

#[test]
fn leave_mcu_above_threshold_non_host_no_change() {
    assert_eq!(
        decide_mode_after_leave(5, &mcu("alice"), false, false, None),
        ModeDecision::NoChange,
    );
}

#[test]
fn stage_channel_leave_never_switches() {
    assert_eq!(
        decide_mode_after_leave(0, &mcu("alice"), true, true, None),
        ModeDecision::NoChange,
    );
}

#[test]
fn stage_host_score_deterministic() {
    let a = stage_host_score("c0ffee", "ab".repeat(32).as_str()).unwrap();
    let b = stage_host_score("c0ffee", "ab".repeat(32).as_str()).unwrap();
    assert_eq!(a, b);
}

#[test]
fn stage_host_score_rejects_bad_hex() {
    assert!(stage_host_score("c0ffee", "nothex").is_none());
    assert!(stage_host_score("c0ffee", "deadbeef").is_none()); // wrong length
}

#[test]
fn select_stage_host_picks_lowest_score() {
    let candidates = vec!["aa".repeat(32), "bb".repeat(32), "cc".repeat(32)];
    let host = select_stage_host("channel-1", &candidates).unwrap();
    // Whichever is lowest XOR to BLAKE3("channel-1") wins. Verify
    // by computing all scores and confirming the picked one is
    // minimum.
    let picked_score = stage_host_score("channel-1", &host).unwrap();
    for c in &candidates {
        let s = stage_host_score("channel-1", c).unwrap();
        assert!(picked_score <= s, "picked host should have lowest score");
    }
}

#[test]
fn select_stage_host_empty_returns_none() {
    assert!(select_stage_host("any", &[]).is_none());
}

#[test]
fn select_stage_host_filters_undecodable() {
    let candidates = vec!["nothex".into(), "aa".repeat(32)];
    let host = select_stage_host("c", &candidates).unwrap();
    assert_eq!(host, "aa".repeat(32));
}

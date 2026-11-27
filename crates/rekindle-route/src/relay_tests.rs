use super::*;

fn key(byte: u8) -> RelayKey {
    [byte; 32]
}

#[test]
fn decode_strips_dummy_entries() {
    let mut dummy = vec![0u8; 16];
    dummy[0] = 0xFF;
    let body = serde_json::to_vec(&vec![vec![1u8, 2, 3], dummy, Vec::new()]).unwrap();
    assert_eq!(decode_relay_pool(&body), vec![vec![1, 2, 3]]);
}

#[test]
fn decode_empty_pool_yields_empty() {
    assert!(decode_relay_pool(&[]).is_empty());
}

#[test]
fn closed_breaker_is_neither_open_nor_half_open() {
    let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
    record_success(&mut map, key(1));
    let health = lookup(&map, &key(1)).unwrap();
    assert!(!health.is_circuit_open());
    assert!(!health.is_half_open());
    assert_eq!(health.consecutive_failures, 0);
}

#[test]
fn three_consecutive_failures_open_breaker() {
    let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
    record_failure(&mut map, key(1));
    record_failure(&mut map, key(1));
    let health = lookup(&map, &key(1)).unwrap();
    assert!(!health.is_circuit_open(), "two failures must not open");
    record_failure(&mut map, key(1));
    let health = lookup(&map, &key(1)).unwrap();
    assert!(health.is_circuit_open(), "three failures must open");
}

#[test]
fn success_after_failures_resets_consecutive_count() {
    let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
    record_failure(&mut map, key(1));
    record_failure(&mut map, key(1));
    record_success(&mut map, key(1));
    record_failure(&mut map, key(1));
    let health = lookup(&map, &key(1)).unwrap();
    assert_eq!(health.consecutive_failures, 1);
    assert!(!health.is_circuit_open());
}

#[test]
fn distinct_relays_isolated() {
    let mut map: HashMap<RelayKey, RelayHealth> = HashMap::new();
    record_failure(&mut map, key(1));
    record_failure(&mut map, key(1));
    record_failure(&mut map, key(1));
    // Relay 2 is untouched.
    assert!(lookup(&map, &key(2)).is_none());
    assert!(lookup(&map, &key(1)).unwrap().is_circuit_open());
}

#[test]
fn rank_is_deterministic_per_target() {
    let candidates = vec![vec![0x10, 0x20], vec![0x30, 0x40], vec![0x50, 0x60]];
    let a = rank_candidates("alice-pubkey-hex", candidates.clone());
    let b = rank_candidates("alice-pubkey-hex", candidates);
    assert_eq!(a, b, "ranking must be stable across calls");
}

#[test]
fn rank_differs_per_target() {
    let candidates = vec![vec![0x11], vec![0x22], vec![0x33], vec![0x44]];
    let a = rank_candidates("alice-pubkey", candidates.clone());
    let b = rank_candidates("zach-pubkey", candidates);
    assert_ne!(a, b);
}

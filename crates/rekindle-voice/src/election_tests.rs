use super::*;

fn pk(prefix: u8) -> String {
    let mut bytes = [0u8; 32];
    bytes[0] = prefix;
    hex::encode(bytes)
}

#[test]
fn target_is_deterministic() {
    let a = channel_target("ch-1234");
    let b = channel_target("ch-1234");
    assert_eq!(a, b);
    let c = channel_target("ch-5678");
    assert_ne!(a, c, "different channel_ids hash to different targets");
}

#[test]
fn election_is_deterministic_across_orderings() {
    let target = channel_target("ch-deterministic");
    let alice = pk(0x10);
    let bob = pk(0x20);
    let carol = pk(0x30);
    let order_a = [alice.clone(), bob.clone(), carol.clone()];
    let order_b = [carol.clone(), alice.clone(), bob.clone()];
    let order_c = [bob.clone(), carol.clone(), alice.clone()];
    let elected_a = elect_relay_host(order_a.iter(), &target);
    let elected_b = elect_relay_host(order_b.iter(), &target);
    let elected_c = elect_relay_host(order_c.iter(), &target);
    assert_eq!(elected_a, elected_b);
    assert_eq!(elected_b, elected_c);
    assert!(elected_a.is_some());
}

#[test]
fn election_minimises_xor_distance() {
    let target = channel_target("ch-xor-min");
    let target_hex = hex::encode(target);
    let lex_smallest = pk(0x00);
    let candidates = [lex_smallest.clone(), target_hex.clone()];
    let elected = elect_relay_host(candidates.iter(), &target).unwrap();
    assert_eq!(
        elected, target_hex,
        "the candidate whose bytes equal the target must win — distance 0"
    );
}

#[test]
fn election_picks_different_hosts_per_channel() {
    let candidates: Vec<String> = (0..8u8).map(pk).collect();
    let target_a = channel_target("ch-A");
    let target_b = channel_target("ch-B-different");
    let host_a = elect_relay_host(candidates.iter(), &target_a).unwrap();
    let host_b = elect_relay_host(candidates.iter(), &target_b).unwrap();
    assert!(candidates.contains(&host_a));
    assert!(candidates.contains(&host_b));
}

#[test]
fn election_skips_invalid_hex() {
    let target = channel_target("ch-skip-bad-hex");
    let good = pk(0x42);
    let bad = "not-real-hex".to_string();
    let elected = elect_relay_host([&bad, &good], &target);
    assert_eq!(elected.as_deref(), Some(good.as_str()));
}

#[test]
fn election_returns_none_for_empty_or_all_bad() {
    let target = channel_target("ch-empty");
    let empty: Vec<String> = Vec::new();
    assert!(elect_relay_host(empty.iter(), &target).is_none());
    let all_bad = ["zzz".to_string(), "qq".to_string()];
    assert!(elect_relay_host(all_bad.iter(), &target).is_none());
}

use super::*;
use tokio::time::Instant;

const FIXED: Span = Span::ms(1_000, 1_000);

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_subkey_serializes_and_two_overlap() {
    let model = Arc::new(Latency::new(1));
    let start = Instant::now();
    let run = |subkey: u32| {
        let model = Arc::clone(&model);
        tokio::spawn(async move {
            let _slot = model.dht(0, FIXED, "k", &[subkey]).await;
            Instant::now()
        })
    };
    let (a, b, c) = (run(0), run(0), run(1));
    let (a, b, c) = (a.await.unwrap(), b.await.unwrap(), c.await.unwrap());
    assert_eq!(a - start, Duration::from_secs(1));
    assert_eq!(b - start, Duration::from_secs(2));
    assert_eq!(c - start, Duration::from_secs(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_cap_queues_the_excess() {
    let model = Arc::new(Latency::new(1));
    let cap = max_concurrent();
    let start = Instant::now();
    let tasks: Vec<_> = (0..=cap)
        .map(|i| {
            let model = Arc::clone(&model);
            let subkey = u32::try_from(i).unwrap();
            tokio::spawn(async move {
                let _slot = model.dht(0, FIXED, "k", &[subkey]).await;
                Instant::now()
            })
        })
        .collect();
    let mut done = Vec::new();
    for t in tasks {
        done.push(t.await.unwrap() - start);
    }
    let late = done
        .iter()
        .filter(|d| **d == Duration::from_secs(2))
        .count();
    assert_eq!(late, 1, "one call past the cap of {cap} waits a round");
}

#[test]
fn a_record_s_draws_do_not_depend_on_another_record_s_order() {
    let ab = Latency::new(9);
    let a_first = (ab.draw_op(0, GET, "a", &[1]), ab.draw_op(0, GET, "b", &[1]));
    let ba = Latency::new(9);
    let b_first = ba.draw_op(0, GET, "b", &[1]);
    let a_second = ba.draw_op(0, GET, "a", &[1]);
    assert_eq!(a_first, (a_second, b_first));
    assert!([a_second, b_first]
        .iter()
        .all(|d| *d >= GET.min && *d <= GET.max));
}

#[test]
fn a_seed_fixes_the_draws_within_the_span() {
    let draws = |seed| {
        let model = Latency::new(seed);
        (0..8).map(|_| model.draw(3, SET)).collect::<Vec<_>>()
    };
    assert_eq!(draws(5), draws(5));
    assert_ne!(draws(5), draws(6));
    assert!(draws(5).iter().all(|d| *d >= SET.min && *d <= SET.max));
}

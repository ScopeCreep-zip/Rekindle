//! Unit tests of the time types (str0m `src/bwe/time.rs`, `mod test`).

use super::*;

#[test]
fn instant_add_duration() {
    let now = Instant::now();

    assert_eq!(
        BweTimestamp::Exact(now) + TimeDelta::from_secs(5),
        BweTimestamp::from(now + Duration::from_secs(5))
    );
    assert_eq!(
        BweTimestamp::Exact(now) + TimeDelta::from_secs(-5),
        BweTimestamp::from(now.checked_sub(Duration::from_secs(5)).unwrap())
    );
    assert_eq!(
        BweTimestamp::Exact(now) + TimeDelta::NegativeInfinity,
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::Exact(now) + TimeDelta::PositiveInfinity,
        BweTimestamp::DistantFuture
    );

    assert_eq!(
        BweTimestamp::DistantPast + TimeDelta::from_secs(5),
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::DistantPast + TimeDelta::from_secs(-5),
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::DistantPast + TimeDelta::NegativeInfinity,
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::DistantPast + TimeDelta::PositiveInfinity,
        BweTimestamp::DistantFuture
    );

    assert_eq!(
        BweTimestamp::DistantFuture + TimeDelta::from_secs(5),
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture + TimeDelta::from_secs(-5),
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture + TimeDelta::NegativeInfinity,
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture + TimeDelta::PositiveInfinity,
        BweTimestamp::DistantFuture
    );
}

#[test]
fn instant_sub_duration() {
    let now = Instant::now();

    assert_eq!(
        BweTimestamp::Exact(now) - TimeDelta::from_secs(5),
        BweTimestamp::from(now.checked_sub(Duration::from_secs(5)).unwrap())
    );
    assert_eq!(
        BweTimestamp::Exact(now) - TimeDelta::from_secs(-5),
        BweTimestamp::from(now + Duration::from_secs(5))
    );
    assert_eq!(
        BweTimestamp::Exact(now) - TimeDelta::NegativeInfinity,
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::Exact(now) - TimeDelta::PositiveInfinity,
        BweTimestamp::DistantPast
    );

    assert_eq!(
        BweTimestamp::DistantPast - TimeDelta::from_secs(5),
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::DistantPast - TimeDelta::from_secs(-5),
        BweTimestamp::DistantPast
    );
    assert_eq!(
        BweTimestamp::DistantPast - TimeDelta::NegativeInfinity,
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantPast - TimeDelta::PositiveInfinity,
        BweTimestamp::DistantPast
    );

    assert_eq!(
        BweTimestamp::DistantFuture - TimeDelta::from_secs(5),
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture - TimeDelta::from_secs(-5),
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture - TimeDelta::NegativeInfinity,
        BweTimestamp::DistantFuture
    );
    assert_eq!(
        BweTimestamp::DistantFuture - TimeDelta::PositiveInfinity,
        BweTimestamp::DistantFuture
    );
}

#[test]
fn instant_sub_instant() {
    let now = Instant::now();

    assert_eq!(
        BweTimestamp::Exact(now) - BweTimestamp::Exact(now),
        TimeDelta::ZERO
    );
    assert_eq!(
        BweTimestamp::Exact(now)
            - BweTimestamp::Exact(now.checked_sub(Duration::from_secs(5)).unwrap()),
        TimeDelta::from_secs(5)
    );
    assert_eq!(
        BweTimestamp::Exact(now) - BweTimestamp::Exact(now + Duration::from_secs(5)),
        TimeDelta::from_secs(-5)
    );
    assert_eq!(
        BweTimestamp::Exact(now) - BweTimestamp::DistantPast,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::Exact(now) - BweTimestamp::DistantFuture,
        TimeDelta::NegativeInfinity
    );

    assert_eq!(
        BweTimestamp::DistantPast - BweTimestamp::Exact(now),
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        BweTimestamp::DistantPast
            - BweTimestamp::Exact(now.checked_sub(Duration::from_secs(5)).unwrap()),
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        BweTimestamp::DistantPast - BweTimestamp::Exact(now + Duration::from_secs(5)),
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        BweTimestamp::DistantPast - BweTimestamp::DistantPast,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::DistantPast - BweTimestamp::DistantFuture,
        TimeDelta::NegativeInfinity
    );

    assert_eq!(
        BweTimestamp::DistantFuture - BweTimestamp::Exact(now),
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::DistantFuture
            - BweTimestamp::Exact(now.checked_sub(Duration::from_secs(5)).unwrap()),
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::DistantFuture - BweTimestamp::Exact(now + Duration::from_secs(5)),
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::DistantFuture - BweTimestamp::DistantPast,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        BweTimestamp::DistantFuture - BweTimestamp::DistantFuture,
        TimeDelta::PositiveInfinity
    );
}

#[test]
fn instant_ord() {
    let now = BweTimestamp::Exact(Instant::now());
    let now_minus_1 = now - TimeDelta::from_secs(1);
    let now_plus_1 = now + TimeDelta::from_secs(1);

    assert!(BweTimestamp::DistantFuture > now_plus_1);
    assert!(BweTimestamp::DistantFuture > now_minus_1);
    assert!(BweTimestamp::DistantFuture > BweTimestamp::DistantPast);

    assert!(now_plus_1 > now_minus_1);
    assert!(now_plus_1 > BweTimestamp::DistantPast);

    assert!(now_minus_1 > BweTimestamp::DistantPast);
}

#[test]
fn duration_ord() {
    assert!(TimeDelta::PositiveInfinity > TimeDelta::from_secs(-2));
    assert!(TimeDelta::PositiveInfinity > TimeDelta::from_secs(2));
    assert!(TimeDelta::PositiveInfinity > TimeDelta::NegativeInfinity);

    assert!(TimeDelta::from_secs(2) > TimeDelta::from_secs(1));
    assert!(TimeDelta::from_secs(2) > TimeDelta::from_secs(-1));
    assert!(TimeDelta::from_secs(2) > TimeDelta::from_secs(-2));
    assert!(TimeDelta::from_secs(2) > TimeDelta::NegativeInfinity);

    assert!(TimeDelta::from_secs(1) > TimeDelta::from_secs(-1));
    assert!(TimeDelta::from_secs(1) > TimeDelta::from_secs(-2));
    assert!(TimeDelta::from_secs(1) > TimeDelta::NegativeInfinity);

    assert!(TimeDelta::from_secs(-1) > TimeDelta::from_secs(-2));
    assert!(TimeDelta::from_secs(-1) > TimeDelta::NegativeInfinity);

    assert!(TimeDelta::from_secs(-2) > TimeDelta::NegativeInfinity);

    assert_eq!(TimeDelta::from_secs(1), Duration::from_secs(1));
    assert!(TimeDelta::from_secs(2) > Duration::from_secs(1));
    assert!(TimeDelta::from_secs(1) < Duration::from_secs(2));
    assert!(TimeDelta::from_secs(-1) < Duration::ZERO);
    assert!(TimeDelta::from_secs(-1) < Duration::from_secs(1));
    assert!(TimeDelta::PositiveInfinity > Duration::from_secs(2));
    assert!(TimeDelta::NegativeInfinity < Duration::from_secs(1));
    assert!(TimeDelta::NegativeInfinity < Duration::ZERO);
}

#[test]
fn duration_add() {
    assert_eq!(
        TimeDelta::PositiveInfinity + TimeDelta::PositiveInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity + TimeDelta::NegativeInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity + TimeDelta::from_secs(-2),
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity + TimeDelta::from_secs(2),
        TimeDelta::PositiveInfinity
    );

    assert_eq!(
        TimeDelta::NegativeInfinity + TimeDelta::PositiveInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity + TimeDelta::NegativeInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity + TimeDelta::from_secs(-2),
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity + TimeDelta::from_secs(2),
        TimeDelta::NegativeInfinity
    );

    assert_eq!(
        TimeDelta::from_secs(1) + TimeDelta::PositiveInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(1) + TimeDelta::NegativeInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(1) + TimeDelta::from_secs(-1),
        TimeDelta::ZERO
    );
    assert_eq!(
        TimeDelta::from_secs(1) + TimeDelta::from_secs(-2),
        TimeDelta::from_secs(-1)
    );
    assert_eq!(
        TimeDelta::from_secs(1) + TimeDelta::from_secs(2),
        TimeDelta::from_secs(3)
    );

    assert_eq!(
        TimeDelta::from_secs(-1) + TimeDelta::PositiveInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(-1) + TimeDelta::NegativeInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(-1) + TimeDelta::from_secs(1),
        TimeDelta::ZERO
    );
    assert_eq!(
        TimeDelta::from_secs(-1) + TimeDelta::from_secs(-2),
        TimeDelta::from_secs(-3)
    );
    assert_eq!(
        TimeDelta::from_secs(-1) + TimeDelta::from_secs(2),
        TimeDelta::from_secs(1)
    );
}

#[test]
fn duration_sub() {
    assert_eq!(
        TimeDelta::PositiveInfinity - TimeDelta::PositiveInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity - TimeDelta::NegativeInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity - TimeDelta::from_secs(-2),
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::PositiveInfinity - TimeDelta::from_secs(2),
        TimeDelta::PositiveInfinity
    );

    assert_eq!(
        TimeDelta::NegativeInfinity - TimeDelta::NegativeInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity - TimeDelta::PositiveInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity - TimeDelta::from_secs(-2),
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::NegativeInfinity - TimeDelta::from_secs(2),
        TimeDelta::NegativeInfinity
    );

    assert_eq!(
        TimeDelta::from_secs(1) - TimeDelta::NegativeInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(1) - TimeDelta::PositiveInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(1) - TimeDelta::from_secs(1),
        TimeDelta::ZERO
    );
    assert_eq!(
        TimeDelta::from_secs(1) - TimeDelta::from_secs(-1),
        TimeDelta::from_secs(2)
    );
    assert_eq!(
        TimeDelta::from_secs(1) - TimeDelta::from_secs(2),
        TimeDelta::from_secs(-1)
    );

    assert_eq!(
        TimeDelta::from_secs(-1) - TimeDelta::NegativeInfinity,
        TimeDelta::PositiveInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(-1) - TimeDelta::PositiveInfinity,
        TimeDelta::NegativeInfinity
    );
    assert_eq!(
        TimeDelta::from_secs(-1) - TimeDelta::from_secs(-1),
        TimeDelta::ZERO
    );
    assert_eq!(
        TimeDelta::from_secs(-1) - TimeDelta::from_secs(1),
        TimeDelta::from_secs(-2)
    );
    assert_eq!(
        TimeDelta::from_secs(-1) - TimeDelta::from_secs(2),
        TimeDelta::from_secs(-3)
    );
}

#[test]
fn super_instant_test() {
    let mut past = BweTimestamp::DistantPast;
    let mut future = BweTimestamp::DistantFuture;
    let now = BweTimestamp::Exact(Instant::now());
    assert!(past < now);
    assert!(now > past);

    assert!(now < future);
    assert!(future > now);

    assert!(past < future);
    assert!(future > past);

    assert!(now == now);
    assert!(past == past);
    assert!(future == future);

    assert!(now != past);
    assert!(now != future);
    assert!(future != past);

    assert!(past - Duration::from_secs(1) == past);
    past -= Duration::from_secs(1);
    assert!(past == past);
    past += Duration::from_secs(1);
    assert!(past == past);

    assert!(future + Duration::from_secs(1) == future);
    assert!(future - Duration::from_secs(1) == future);
    future -= Duration::from_secs(1);
    assert!(future == future);
    future += Duration::from_secs(1);
    assert!(future == future);
}

//! The unlocked identity's new-route budget and own-route death counts
//! (plan RV1, owner decision 2026-11-22; threat model L2 bounded exposure).
//!
//! At unlock the identity's bucket is read from its database (a full one
//! at its first unlock) and handed to the route owner, which spends a token
//! for every route whose blob leaves the device and persists each spend
//! here. A spent bucket keeps the class down; the Host tells every frontend
//! ([`NetworkEvent::RoutePaused`]) and the status snapshot carries it. The
//! per-class death counts are compared with the baseline and an excess is
//! a warning only ([`NetworkEvent::RouteDeathsAboveBaseline`]), never an
//! automatic reaction.
//!
//! Debug builds only: `REKINDLE_ROUTE_BUDGET_CAPACITY=<n>` seeds the attach
//! with a bucket of `n` tokens and capacity `n` (at most the default), so
//! the real-app suite can see `RoutePaused { Exhausted }` at unlock with
//! `0`. The seeded attach neither reads nor writes the stored row (its save
//! drops each spend), so the identity's persisted budget is untouched. The
//! name and the code reading it exist only under `cfg(debug_assertions)`
//! (`cargo xtask check-log-scrub` keeps every mention there).

use std::sync::Arc;
use std::time::Duration;

use rekindle_db::repo::route_budget::BudgetKind;
use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{
    RouteBudgetState, RouteClass, RoutePauseCause, RoutePort, SaveBudget,
};
use rekindle_route::budget::{
    DeathRate, RouteBudget, BUDGET_CAPACITY, DEATH_WARN_ABOVE, DEATH_WINDOW,
};
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::subscription_events::{NetworkEvent, OwnRouteClass, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use crate::daemon::dispatch::DaemonContext;

/// How often the Host reads the per-class death counts.
const DEATH_CHECK: Duration = Duration::from_secs(30);

/// Unlock: attach the identity `owner`'s budget to the route owner and
/// start telling the pause and the death warnings on `scope`. A budget that
/// cannot be read is logged and attached as unreadable: nothing is paid
/// for, every class stays down and the pause shows its cause `Unreadable`
/// (fail-closed and visible, never a full budget in its place).
pub(crate) async fn attach(ctx: &DaemonContext, scope: &Arc<SessionScope>, owner: &str) {
    attach_code_rolls(ctx, owner).await;
    let Some(transport) = ctx.transport.read().clone() else {
        return;
    };
    let routes = transport.routes();
    let down_routes = Arc::clone(&routes);
    let down_friends = super::adapters::friends::HostFriends::of(ctx);
    scope.spawn_with_token_or_drop("own route down events", move |stop| {
        super::route_down::tell_down(down_routes, down_friends, stop)
    });
    if !attach_seeded(ctx, routes.as_ref()) && !attach_stored(ctx, routes.as_ref(), owner).await {
        return;
    }
    let friends = super::adapters::friends::HostFriends::of(ctx);
    let paused_routes = Arc::clone(&routes);
    let paused_friends = friends.clone();
    scope.spawn_with_token_or_drop("route pause events", move |stop| {
        tell_paused(paused_routes, paused_friends, stop)
    });
    scope.spawn_with_token_or_drop("route death counts", move |stop| {
        watch_deaths(routes, friends, stop)
    });
}

/// The debug-only variable that seeds the attach (plan RV1 real-app suite).
#[cfg(debug_assertions)]
const CAPACITY_VAR: &str = "REKINDLE_ROUTE_BUDGET_CAPACITY";

/// The seed a `CAPACITY_VAR` value names: unset is `None` (the stored
/// budget), else a whole number of tokens no larger than the default
/// capacity (the seed only shrinks the bucket), or `Err` naming what is
/// wrong.
#[cfg(debug_assertions)]
fn capacity_from(value: Option<&str>) -> Result<Option<u32>, String> {
    let Some(text) = value else {
        return Ok(None);
    };
    match text.trim().parse::<u32>() {
        Ok(n) if n <= BUDGET_CAPACITY => Ok(Some(n)),
        _ => Err(format!(
            "{CAPACITY_VAR} must be a whole number of tokens from 0 to {BUDGET_CAPACITY}, got {text:?}"
        )),
    }
}

/// Debug builds: attach a bucket seeded from `CAPACITY_VAR` when it is
/// set, its spends dropped so the stored row is never overwritten; `false`
/// when unset. A malformed value stops the process: unlocking on the full
/// stored budget would silently defeat the run it was set for.
#[cfg(debug_assertions)]
fn attach_seeded(ctx: &DaemonContext, routes: &dyn RoutePort) -> bool {
    use std::io::Write as _;
    let value = std::env::var(CAPACITY_VAR).ok();
    let capacity = match capacity_from(value.as_deref()) {
        Ok(Some(capacity)) => capacity,
        Ok(None) => return false,
        Err(e) => {
            let _ = writeln!(std::io::stderr().lock(), "route budget: {e}");
            std::process::exit(2);
        }
    };
    tracing::warn!(
        capacity,
        "debug build: route budget seeded from {CAPACITY_VAR}; the stored budget is neither read nor written"
    );
    let state = RouteBudget::seeded(capacity, ctx.clock.now_ms()).state();
    let clock = Arc::clone(&ctx.clock);
    let drop_spend: rekindle_records::port::SaveBudget = Arc::new(|_: RouteBudgetState| {});
    routes.attach_budget(state, Arc::new(move || clock.now_ms()), drop_spend);
    true
}

/// Release builds: the stored budget, always.
#[cfg(not(debug_assertions))]
fn attach_seeded(_: &DaemonContext, _: &dyn RoutePort) -> bool {
    false
}

/// Attach the identity `owner`'s stored budget (a full one, saved, at its
/// first unlock); `false` when no database is open.
async fn attach_stored(ctx: &DaemonContext, routes: &dyn RoutePort, owner: &str) -> bool {
    let Some((state, save)) = stored(ctx, owner, BudgetKind::Routes).await else {
        return false;
    };
    match state {
        Some(state) => {
            let clock = Arc::clone(&ctx.clock);
            routes.attach_budget(state, Arc::new(move || clock.now_ms()), save);
        }
        None => routes.attach_unreadable_budget(),
    }
    true
}

/// Unlock: the code slot's roll budget (plan RV2e) from the identity
/// `owner`'s database, persisted as the route budget is, so a restart
/// never refills it. With no database, or a row that cannot be read, no
/// roll is paid for (fail-closed, never a full bucket in its place).
async fn attach_code_rolls(ctx: &DaemonContext, owner: &str) {
    let friends = super::adapters::friends::HostFriends::of(ctx);
    friends.set_code_rolls(None);
    if let Some((Some(state), save)) = stored(ctx, owner, BudgetKind::CodeRolls).await {
        friends.set_code_rolls(Some((RouteBudget::from(state), save)));
    }
}

/// The identity `owner`'s stored bucket `kind` and the save each spend
/// calls: a full bucket, saved, at its first unlock, `None` in place of
/// the state when its row cannot be read (logged); `None` when no
/// database is open.
async fn stored(
    ctx: &DaemonContext,
    owner: &str,
    kind: BudgetKind,
) -> Option<(Option<RouteBudgetState>, SaveBudget)> {
    let db = ctx.db.current().ok()?;
    let now = ctx.clock.now_ms();
    let o = owner.to_string();
    let row = db
        .call(move |conn| rekindle_db::repo::route_budget::load(conn, &o, kind))
        .await;
    let save_db = db.clone();
    let save_owner = owner.to_string();
    let save: SaveBudget = Arc::new(move |s: RouteBudgetState| {
        let o = save_owner.clone();
        save_db.fire("route budget", move |conn| {
            rekindle_db::repo::route_budget::save(
                conn,
                &o,
                kind,
                (s.tokens, s.refilled_ms, s.week_start_ms, s.week_spent),
            )
        });
    });
    let state = match row {
        Ok(Some((tokens, refilled_ms, week_start_ms, week_spent))) => Some(RouteBudgetState {
            tokens,
            refilled_ms,
            week_start_ms,
            week_spent,
            capacity: BUDGET_CAPACITY,
        }),
        Ok(None) => {
            let full = RouteBudget::full(now).state();
            save(full);
            Some(full)
        }
        Err(e) => {
            tracing::error!(error = %e, ?kind, "budget unreadable; nothing is paid from it");
            None
        }
    };
    Some((state, save))
}

/// Lock: the identity's budget is no longer the route owner's to spend.
pub(crate) fn detach(routes: &dyn RoutePort) {
    routes.detach_budget();
}

/// Tell every frontend whether a class is down for the budget: its state
/// now, then each change, until `stop`.
async fn tell_paused(
    routes: Arc<dyn RoutePort>,
    friends: super::adapters::friends::HostFriends,
    stop: CancellationToken,
) {
    let mut paused = routes.own_paused();
    loop {
        let event = match *paused.borrow_and_update() {
            Some(cause @ RoutePauseCause::Exhausted) => {
                tracing::warn!("inbound route paused after repeated failures: possible attack");
                NetworkEvent::RoutePaused { cause }
            }
            Some(cause @ RoutePauseCause::Unreadable) => {
                tracing::error!("inbound route paused: the stored route budget is unreadable");
                NetworkEvent::RoutePaused { cause }
            }
            None => NetworkEvent::RouteResumed {},
        };
        friends.publish(SubscriptionEvent::Network(event));
        let changed = stop.run_until_cancelled(paused.changed()).await;
        if !matches!(changed, Some(Ok(()))) {
            return;
        }
    }
}

/// Compare each class's own-route deaths with the baseline every
/// [`DEATH_CHECK`] and warn when they rise above it, until `stop`.
async fn watch_deaths(
    routes: Arc<dyn RoutePort>,
    friends: super::adapters::friends::HostFriends,
    stop: CancellationToken,
) {
    let classes = [
        (RouteClass::General, OwnRouteClass::General),
        (RouteClass::Media, OwnRouteClass::Media),
    ];
    let mut rates: [DeathRate; 2] =
        classes.map(|(class, _)| DeathRate::new(routes.own_deaths(class)));
    loop {
        if stop
            .run_until_cancelled(tokio::time::sleep(DEATH_CHECK))
            .await
            .is_none()
        {
            return;
        }
        let now = tokio::time::Instant::now();
        for ((class, shown), rate) in classes.iter().zip(rates.iter_mut()) {
            if let Some(deaths) = rate.observe(routes.own_deaths(*class), now) {
                tracing::warn!(
                    ?class,
                    deaths = deaths.total,
                    young = deaths.young,
                    "own-route deaths above the baseline"
                );
                friends.publish(SubscriptionEvent::Network(
                    NetworkEvent::RouteDeathsAboveBaseline {
                        class: *shown,
                        deaths: deaths.total,
                        young: deaths.young,
                        window_secs: DEATH_WINDOW.as_secs(),
                        baseline: DEATH_WARN_ABOVE,
                    },
                ));
            }
        }
    }
}

#[cfg(all(test, debug_assertions))]
#[path = "route_budget_tests.rs"]
mod tests;

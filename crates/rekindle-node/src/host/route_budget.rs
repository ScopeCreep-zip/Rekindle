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

use std::sync::Arc;
use std::time::Duration;

use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{RouteBudgetState, RouteClass, RoutePort};
use rekindle_route::budget::{DeathRate, RouteBudget, DEATH_WARN_ABOVE, DEATH_WINDOW};
use rekindle_transport::operations::friend::FriendHost;
use rekindle_types::subscription_events::{NetworkEvent, OwnRouteClass, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use crate::daemon::dispatch::DaemonContext;

/// How often the Host reads the per-class death counts.
const DEATH_CHECK: Duration = Duration::from_secs(30);

/// Unlock: attach the identity `owner`'s budget to the route owner and
/// start telling the pause and the death warnings on `scope`. A budget that
/// cannot be read attaches empty: the classes stay down and the pause
/// shows (never an unbounded redraw).
pub(crate) async fn attach(ctx: &DaemonContext, scope: &Arc<SessionScope>, owner: &str) {
    let Some(transport) = ctx.transport.read().clone() else {
        return;
    };
    let routes = transport.routes();
    let Ok(db) = ctx.db.current() else {
        return;
    };
    let now = ctx.clock.now_ms();
    let o = owner.to_string();
    let row = db
        .call(move |conn| rekindle_db::repo::route_budget::load(conn, &o))
        .await;
    let save_db = db.clone();
    let save_owner = owner.to_string();
    let save: rekindle_records::port::SaveBudget = Arc::new(move |s: RouteBudgetState| {
        let o = save_owner.clone();
        save_db.fire("route budget", move |conn| {
            rekindle_db::repo::route_budget::save(
                conn,
                &o,
                (s.tokens, s.refilled_ms, s.week_start_ms, s.week_spent),
            )
        });
    });
    let state = match row {
        Ok(Some((tokens, refilled_ms, week_start_ms, week_spent))) => RouteBudgetState {
            tokens,
            refilled_ms,
            week_start_ms,
            week_spent,
        },
        Ok(None) => {
            let full = RouteBudget::full(now).state();
            save(full);
            full
        }
        Err(e) => {
            tracing::warn!(error = %e, "route budget unreadable; routes stay down");
            RouteBudgetState {
                tokens: 0,
                refilled_ms: now,
                week_start_ms: now,
                week_spent: 0,
            }
        }
    };
    let clock = Arc::clone(&ctx.clock);
    routes.attach_budget(state, Arc::new(move || clock.now_ms()), save);
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
        let now = *paused.borrow_and_update();
        if now {
            tracing::warn!("inbound route paused after repeated failures: possible attack");
        }
        friends.publish(SubscriptionEvent::Network(NetworkEvent::RoutePaused {
            paused: now,
        }));
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
                tracing::warn!(?class, deaths, "own-route deaths above the baseline");
                friends.publish(SubscriptionEvent::Network(
                    NetworkEvent::RouteDeathsAboveBaseline {
                        class: *shown,
                        deaths,
                        window_secs: DEATH_WINDOW.as_secs(),
                        baseline: DEATH_WARN_ABOVE,
                    },
                ));
            }
        }
    }
}

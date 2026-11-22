//! The route owner's accounts (plan RV1, owner decision 2026-11-22; threat
//! model L2 bounded exposure): the unlocked identity's new-route budget,
//! why a class is down without a new route, and the per-class death counts
//! the Host compares with the baseline. Split from [`crate::own_routes`],
//! which allocates; this module only decides whether an allocation is paid
//! for and counts what died.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{
    OwnDeaths, RouteBudgetState, RouteClass, RoutePauseCause, SaveBudget, WallMs,
};
use tokio::sync::watch;

use crate::budget::{RouteBudget, YOUNG_ROUTE_AGE};
use crate::ordered_watch::OrderedWatch;

/// The unlocked identity's new-route budget ([`crate::budget`]).
struct BudgetHook {
    state: RouteBudget,
    wall: WallMs,
    save: SaveBudget,
}

/// Whose budget a new route is paid from.
enum Budget {
    /// No identity is unlocked: no route's blob leaves the device, so a
    /// route allocated now is paid for at the next unlock.
    Locked,
    /// The unlocked identity's budget.
    Attached(BudgetHook),
    /// The unlocked identity's stored budget could not be read: nothing is
    /// paid for (fail-closed, never a full budget in its place).
    Unreadable,
}

/// The budget, the pause and the death counts of one route owner.
pub(crate) struct RouteAccounts {
    budget: Mutex<Budget>,
    /// Why a class is down without a new route, if one is: the Host shows
    /// it.
    paused: OrderedWatch<Option<RoutePauseCause>>,
    /// Own-route deaths per class since the node started (General, Media).
    deaths: [AtomicU32; 2],
    /// Of those, the routes that died younger than [`YOUNG_ROUTE_AGE`].
    young_deaths: [AtomicU32; 2],
}

impl RouteAccounts {
    pub(crate) fn new() -> Self {
        Self {
            budget: Mutex::new(Budget::Locked),
            paused: OrderedWatch::new(None),
            deaths: Default::default(),
            young_deaths: Default::default(),
        }
    }

    /// Unlock: the identity's budget `state`, read with `wall` and
    /// persisted through `save`.
    pub(crate) fn attach(&self, state: RouteBudgetState, wall: WallMs, save: SaveBudget) {
        *self.budget.lock() = Budget::Attached(BudgetHook {
            state: RouteBudget::from(state),
            wall,
            save,
        });
    }

    /// Unlock with a stored budget that could not be read.
    pub(crate) fn attach_unreadable(&self) {
        *self.budget.lock() = Budget::Unreadable;
    }

    /// Lock: the identity's budget is no longer ours to spend.
    pub(crate) fn detach(&self) {
        *self.budget.lock() = Budget::Locked;
    }

    /// Spend one token for a new route of ours; `true` while locked (the
    /// route is not published until an unlock pays for it).
    pub(crate) fn spend(&self) -> bool {
        let mut guard = self.budget.lock();
        let hook = match &mut *guard {
            Budget::Locked => return true,
            Budget::Unreadable => return false,
            Budget::Attached(hook) => hook,
        };
        let now = (hook.wall)();
        if !hook.state.try_spend(now) {
            return false;
        }
        (hook.save)(hook.state.state());
        drop(guard);
        self.paused.send_replace(None);
        true
    }

    /// Spend a token for a route whose blob leaves the device but which the
    /// owner does not hold (a relay offer): `false` when the budget is spent
    /// or unreadable, or no identity is unlocked.
    pub(crate) fn spend_external(&self) -> bool {
        !matches!(*self.budget.lock(), Budget::Locked) && self.spend()
    }

    /// Whether a new route of ours could be paid for now.
    pub(crate) fn can_spend(&self) -> bool {
        match &*self.budget.lock() {
            Budget::Locked => true,
            Budget::Unreadable => false,
            Budget::Attached(hook) => hook.state.can_spend((hook.wall)()),
        }
    }

    /// A class could not be paid for: show why.
    pub(crate) fn mark_paused(&self) -> RoutePauseCause {
        let cause = match *self.budget.lock() {
            Budget::Unreadable => RoutePauseCause::Unreadable,
            Budget::Locked | Budget::Attached(_) => RoutePauseCause::Exhausted,
        };
        self.paused.send_replace(Some(cause));
        cause
    }

    /// Why a class is down without a new route, to follow.
    pub(crate) fn paused(&self) -> watch::Receiver<Option<RoutePauseCause>> {
        self.paused.subscribe()
    }

    /// A route of `class` died at `age`.
    pub(crate) fn record_death(&self, class: RouteClass, age: Duration) {
        self.deaths[class_index(class)].fetch_add(1, Ordering::SeqCst);
        if age < YOUNG_ROUTE_AGE {
            self.young_deaths[class_index(class)].fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Own-route deaths of `class` since the node started.
    pub(crate) fn deaths(&self, class: RouteClass) -> OwnDeaths {
        OwnDeaths {
            total: self.deaths[class_index(class)].load(Ordering::SeqCst),
            young: self.young_deaths[class_index(class)].load(Ordering::SeqCst),
        }
    }
}

fn class_index(class: RouteClass) -> usize {
    match class {
        RouteClass::General => 0,
        RouteClass::Media => 1,
    }
}

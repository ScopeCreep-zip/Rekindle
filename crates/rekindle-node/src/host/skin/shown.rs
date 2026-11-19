//! The role-tag grounds of the skin this device shows (plan E7.2 slice 4,
//! E7.3 "role tags"): `rekindle_skin::role_tag` reads every built-in skin's
//! grounds, and an imported skin's only through this slot. The Host sets it
//! each time it resolves the shown skin ([`super::serve`]), and when the
//! grounds change, re-sends the roles whose tag colours change with them
//! (`GovernanceEvent::RolesChanged`), so a window never keeps a colour
//! worked out for another skin.

use std::sync::Arc;

use rekindle_skin::role_tag::SkinGrounds;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::governance::role_ops::role_displays;

/// The Host's one slot (`DaemonContext::skin_grounds`).
#[derive(Default)]
pub(crate) struct ShownGrounds {
    inner: parking_lot::RwLock<Option<Arc<SkinGrounds>>>,
}

impl ShownGrounds {
    /// The shown skin's grounds; `None` before the first resolution.
    pub(crate) fn get(&self) -> Option<Arc<SkinGrounds>> {
        self.inner.read().clone()
    }

    /// Hold `grounds`.
    fn replace(&self, grounds: SkinGrounds) -> Change {
        let mut held = self.inner.write();
        if held.as_deref() == Some(&grounds) {
            return Change::Same;
        }
        Change::Was(held.replace(Arc::new(grounds)))
    }
}

/// What holding new grounds changed.
enum Change {
    Same,
    /// The grounds held before (none before the first resolution).
    Was(Option<Arc<SkinGrounds>>),
}

/// Record the shown skin's grounds and re-send every community's roles
/// whose tag colours they change.
pub(crate) fn note(ctx: &DaemonContext, grounds: SkinGrounds) {
    let Change::Was(before) = ctx.skin_grounds.replace(grounds) else {
        return;
    };
    let after = ctx.skin_grounds.get();
    let ids: Vec<String> = ctx.communities.map().read().keys().cloned().collect();
    for community in ids {
        let Some(gov) = ctx.communities.governance_state(&community) else {
            continue;
        };
        let roles = role_displays(&gov, after.as_deref());
        let was = role_displays(&gov, before.as_deref());
        if roles
            .iter()
            .map(|r| r.tag_colours)
            .ne(was.iter().map(|r| r.tag_colours))
        {
            ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::RolesChanged { community, roles },
            ));
        }
    }
}

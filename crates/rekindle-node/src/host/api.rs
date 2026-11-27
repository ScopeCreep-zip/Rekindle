//! The host's side of [`rekindle_ipc::HostApi`] (plan E1.1): the trait,
//! [`Caller`] and [`ConnId`] live in `rekindle-ipc`, so frontends name them
//! without the node.

use rekindle_events::EventStream;
use rekindle_ipc::error::IpcError;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::subscription_events::{Cursor, SubscriptionEvent};

use super::gate::IntentOwner;

impl super::Host {
    /// Where the unlocked identity's own-records stage is (plan E2.5 FC3),
    /// for a window's first read; changes follow as
    /// `NetworkEvent::OwnRecordsChanged`.
    #[must_use]
    pub fn own_records_stage(&self) -> rekindle_transport::operations::friend::OwnRecordsStage {
        self.ctx.friend_stores.own_records.get()
    }

    /// Journal `event` as one of the session's standing events (plan E7.1:
    /// a security alert the shell raises at login): every window that opens
    /// later is handed it ([`Self::standing`]).
    pub fn publish_standing(
        &self,
        event: rekindle_types::subscription_events::SubscriptionEvent,
    ) -> Cursor {
        self.ctx.journal.publish_standing(event)
    }

    /// Journal `event` as the current value of the session state `key`
    /// (plan C7.29: the voice roster): a window that opens later is handed
    /// the latest value, then the live changes.
    pub fn publish_state(
        &self,
        key: &str,
        event: rekindle_types::subscription_events::SubscriptionEvent,
    ) -> Cursor {
        self.ctx.journal.publish_state(key, event)
    }

    /// The session state `key` ended: no later window is handed it.
    pub fn clear_state(&self, key: &str) {
        self.ctx.journal.clear_state(key);
    }

    /// The session's standing events with their cursors: the snapshot a GUI
    /// window that just opened is handed before its live events.
    #[must_use]
    pub fn standing(&self) -> Vec<rekindle_types::subscription_events::Sequenced> {
        self.ctx.journal.standing()
    }

    /// The complete set of reviews the backend holds for the unlocked
    /// identity (plan E3.2 S7d-keys.10): unexpired, neither committed,
    /// cancelled nor replaced, each under its publication id; empty while
    /// locked. A window is handed it with the standing events on every
    /// subscribe and resume, so a review raised before it subscribed (the
    /// unlock barrier's `ReEnrolDevice`) still reaches it.
    #[must_use]
    pub fn held_reviews(&self) -> SubscriptionEvent {
        SubscriptionEvent::HeldReviews(super::held_reviews_of(&self.ctx))
    }

    /// Recheck a queued backend review just before delivering it to the shell.
    #[must_use]
    pub fn holds_review(
        &self,
        hold_id: rekindle_types::intent::IntentId,
        intent_id: rekindle_types::intent::IntentId,
    ) -> bool {
        self.ctx.require_owner().is_ok_and(|identity| {
            let owner = IntentOwner {
                conn: None,
                identity,
            };
            self.ctx
                .intents
                .holds_review(hold_id, intent_id, &owner, self.ctx.clock.now_ms())
        })
    }

    /// Stage a prekey cutover at the point immediately before the
    /// identity transaction, for the simulated crash/restart scenario.
    ///
    /// # Panics
    /// The simulated Host must be unlocked and its test vault writable.
    #[cfg(feature = "netsim-entry")]
    pub fn netsim_stage_prekey_cutover(&self) {
        let guard = self.ctx.vault.lock();
        let vault = guard.as_ref().expect("the simulated identity is unlocked");
        rekindle_vault::typed::owner_keys::stage_prekey_owner(
            vault,
            &rekindle_vault::typed::owner_keys::OwnerEntry {
                record_key: "simulated-provisional-record".to_owned(),
                keypair: "simulated-provisional-owner".to_owned(),
            },
        )
        .expect("stage the replacement owner");
    }

    /// The committed prekey record and whether a staged owner remains.
    ///
    /// # Panics
    /// The simulated Host has no unlocked prekey owner or its test vault is unreadable.
    #[cfg(feature = "netsim-entry")]
    pub fn netsim_prekey_cutover_state(&self) -> (String, bool) {
        use rekindle_vault::{
            typed::owner_keys::{load_owner, OwnerKey},
            VaultKey,
        };
        let guard = self.ctx.vault.lock();
        let vault = guard.as_ref().expect("the simulated identity is unlocked");
        let owner = load_owner(vault, OwnerKey::Prekeys)
            .unwrap()
            .expect("prekey owner saved");
        (
            owner.record_key,
            vault.get(&VaultKey::PendingPrekeyOwner).unwrap().is_some(),
        )
    }

    /// A fresh `SystemAlert` id, for an alert the shell raises through
    /// the Host ([`Self::publish_standing`]).
    #[must_use]
    pub fn alert_id(&self) -> String {
        self.ctx.alert_id()
    }
}

#[async_trait::async_trait]
impl HostApi for super::Host {
    async fn call(&self, caller: Caller, req: IpcRequest) -> IpcResponse {
        self.ctx.call(req, &caller).await
    }

    async fn subscribe(&self, from: Option<Cursor>) -> Result<EventStream, IpcError> {
        Ok(self.ctx.journal.subscribe(from))
    }
}

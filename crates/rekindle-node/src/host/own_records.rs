//! The own-records stage (plan E1.1): the unlocked identity's mailbox,
//! prekey record (plan E2.4, `super::own_prekeys`), profile, friend list
//! and account, on the record port, on both hosts. Opened when the identity has them, created when it does
//! not (a new identity), so create needs no transport of its own.
//!
//! Every record with an owner keypair of its own is held by one rule
//! (`super::owned_record`): saved vault entry first, compared with the
//! identity row at unlock and repaired from the vault after an interrupted
//! write. The keys land in the session projection, and the current general
//! route is published to the profile and the mailbox; a route that lands
//! later is the transport's route publisher's. Our prekey record is
//! reconciled and filled before the profile names it, and only then is the
//! prekey maintenance told it may start (`prekeys_ready`).

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_db::repo::identity::OwnedRecord;
use rekindle_db::DbHandle;
use rekindle_protocol::dht::{account, friends, mailbox, profile};
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, RouteClass};
use rekindle_transport::{Session, TransportNode};
use rekindle_vault::VaultHandle;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::owned_record::{Owner, StoredRecord};
use super::shell_hooks::{OwnRecordKeys, ShellHooks};

/// What the stage needs, owned so it runs in the unlock scope.
pub(crate) struct OwnRecords {
    pub transport: Arc<TransportNode>,
    pub db: Arc<DbHandle>,
    pub vault: VaultHandle,
    pub session: Arc<RwLock<Option<Session>>>,
    pub shell: Option<Arc<dyn ShellHooks>>,
    /// The Host's generator (C9.2): this device's id is minted with it
    /// (the prekeys are the prekey maintenance's, plan E2.4).
    pub rng: Arc<dyn rekindle_utils::RandomSource>,
    /// Our presence: its message goes into the profile, and its publisher
    /// writes our status once the profile exists.
    pub presence: super::own_presence::OwnPresence,
    /// Where the profile's session lease is kept, so a rotation (block)
    /// can release it (`DaemonContext.own_profile_lease`).
    pub profile_lease: Arc<parking_lot::Mutex<Option<LeaseId>>>,
    /// Where our prekey record's session lease is kept
    /// (`DaemonContext.own_prekey_lease`): the prekey maintenance and the
    /// refills write through it.
    pub prekey_lease: Arc<parking_lot::Mutex<Option<LeaseId>>>,
    /// The one writer at a time of our prekey record
    /// (`DaemonContext.own_prekey_writes`): the stage holds it from its
    /// reconcile through its later slots.
    pub prekey_writes: Arc<tokio::sync::Mutex<()>>,
    /// Sent once our prekey record is held, reconciled and filled: the
    /// prekey maintenance waits for it (audit D-7), so it never mints
    /// beside the stage. Dropped unsent when the record is not held.
    pub prekeys_ready: Option<tokio::sync::oneshot::Sender<()>>,
    /// The friend operations' Host: the stage reconciles our prekey record
    /// and publishes its first set through it before the profile names it.
    pub friends: super::adapters::friends::HostFriends,
    pub public_key: String,
    pub display_name: String,
    pub secret: Zeroizing<[u8; 32]>,
}

/// The records' keys as the identity row and the vault hold them.
pub(super) struct Stored {
    profile: StoredRecord,
    friend_list: StoredRecord,
    account: StoredRecord,
    mailbox: Option<String>,
    pub(super) prekeys: StoredRecord,
    pub(super) device_id: Option<String>,
}

impl OwnRecords {
    /// Run the stage until it is done or `stop` (the unlock ends).
    pub(crate) async fn run(mut self, stop: CancellationToken) {
        // Readiness is the record port's own gate; the general route is
        // wanted from node-ready (C7.9b), so it is usually here already.
        let routes = self.transport.routes();
        let Some(route_blob) = stop
            .run_until_cancelled(routes.available_own(RouteClass::General))
            .await
        else {
            return;
        };
        let Ok(pool) = self.transport.require_records() else {
            tracing::warn!("own records: no record pool");
            return;
        };
        let stored = match self.stored().await {
            Ok(stored) => stored,
            Err(e) => {
                tracing::error!(error = %e, "own records: the stored keys could not be read");
                return;
            }
        };
        let identity = rekindle_crypto::Identity::from_secret_bytes(&self.secret);
        let identity_keypair = rekindle_crypto::keypair_text::keypair_text(
            &identity.public_key_bytes(),
            identity.secret_key_bytes(),
        );
        let mut keys = OwnRecordKeys::default();
        let mut owners = (
            stored.profile.matching_keypair().map(ToString::to_string),
            stored
                .friend_list
                .matching_keypair()
                .map(ToString::to_string),
        );

        // The prekey maintenance may start once our record is held,
        // reconciled and filled (or not held, and the sender goes unsent).
        let ready = self.prekeys_ready.take();
        let this = &self;
        let pool = pool.as_ref();
        // The records that do not depend on one another are written at
        // once (plan E2.B): the mailbox, the profile and the friend list
        // beside the prekey record; only the profile's subkey 5, which
        // names the record, waits for its first-contact part.
        let profile = async {
            let prekeys = async {
                let held = this.prekeys(pool, &stored).await;
                if let Some(ready) = ready {
                    if this.prekey_lease.lock().is_some() {
                        let _ = ready.send(());
                    }
                }
                held
            };
            let ((device_list, later, writes), profile) =
                tokio::join!(prekeys, this.profile(pool, &stored, &route_blob));
            // The account, read by no one else, is written beside the
            // device list rather than with the first writes: those already
            // fill Veilid's 16 concurrent DHT operations (11 prekey writes,
            // the profile's 3 fields, the mailbox route and the friend
            // list), and one more would wait a whole set latency for a slot
            // ahead of the record the profile names.
            let device_list = async {
                if let Some((key, _)) = &profile {
                    this.device_list(pool, key, &device_list).await;
                }
            };
            tokio::join!(device_list, this.account(pool, &stored));
            // The later slots after the profile names the record, so they
            // never hold its writes back under the DHT cap.
            this.publish_later(later, writes);
            profile
        };
        let joined = stop
            .run_until_cancelled(async {
                tokio::join!(
                    this.mailbox(pool, stored.mailbox.clone(), &identity_keypair, &route_blob),
                    profile,
                    this.friend_list(pool, &stored),
                )
            })
            .await;
        let Some((mailbox, profile, friend_list)) = joined else {
            return;
        };
        keys.mailbox = mailbox;
        if let Some((key, owner)) = profile {
            keys.profile = Some(key);
            owners.0 = Some(owner);
        }
        if let Some((key, owner)) = friend_list {
            keys.friend_list = Some(key);
            owners.1 = Some(owner);
        }
        self.project(&keys, owners);
        // The status, now that the projection names the profile (a created
        // identity had none until this stage).
        self.presence.republish();
        if let Some(shell) = &self.shell {
            shell.own_records(&keys);
        }
        tracing::info!("own records held");
    }

    async fn stored(&self) -> Result<Stored, String> {
        let db = self.db.current().map_err(|e| e.to_string())?;
        let owner = self.public_key.clone();
        let columns = db
            .call(move |conn| {
                Ok(rekindle_db::repo::identity::login_row(conn, &owner)?.map(|(_, c)| c))
            })
            .await
            .map_err(|e| e.to_string())?;
        let columns = columns.ok_or("the identity database has no identity row")?;
        let guard = self.vault.lock();
        let vault = guard.as_ref().ok_or("the vault is locked")?;
        Ok(Stored {
            profile: StoredRecord::load(vault, OwnedRecord::Profile, columns.profile)?,
            friend_list: StoredRecord::load(vault, OwnedRecord::FriendList, columns.friend_list)?,
            account: StoredRecord::load(vault, OwnedRecord::Account, columns.account)?,
            mailbox: columns.mailbox,
            prekeys: StoredRecord::load(vault, OwnedRecord::Prekeys, columns.prekeys)?,
            device_id: columns.device_id,
        })
    }

    /// The owned-record rule's view of this identity.
    pub(super) fn owner(&self) -> Owner<'_> {
        Owner {
            vault: &self.vault,
            db: &self.db,
            public_key: &self.public_key,
        }
    }

    async fn mailbox(
        &self,
        pool: &dyn RecordPort,
        existing: Option<String>,
        identity_keypair: &str,
        route_blob: &[u8],
    ) -> Option<String> {
        let key = if let Some(key) = existing {
            if let Err(e) = mailbox::open_mailbox_writable(pool, &key, identity_keypair).await {
                tracing::warn!(error = %e, "own records: mailbox not reopened");
                return None;
            }
            key
        } else {
            let (_, key) = mailbox::create_mailbox(pool, identity_keypair)
                .await
                .map_err(|e| tracing::warn!(error = %e, "own records: mailbox not created"))
                .ok()?;
            let db = self.db.current().ok()?;
            let (pk, k) = (self.public_key.clone(), key.clone());
            if let Err(e) = db
                .call(move |conn| rekindle_db::repo::identity::set_mailbox_key(conn, &pk, &k))
                .await
            {
                tracing::error!(error = %e, "own records: mailbox key not saved");
            }
            key
        };
        match mailbox::update_mailbox_route(pool, &key, route_blob).await {
            Ok(outcome) if outcome.missed() => {
                tracing::warn!(
                    ?outcome,
                    "own records: mailbox route not stored at consensus"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "own records: mailbox route not written"),
        }
        Some(key)
    }

    async fn profile(
        &self,
        pool: &dyn RecordPort,
        stored: &Stored,
        route_blob: &[u8],
    ) -> Option<(String, String)> {
        let status_message = self.presence.view().status_message;
        let fields = profile::ProfileFields {
            display_name: &self.display_name,
            status_message: &status_message,
            prekey_records: None,
            route_blob,
        };
        // An existing profile is re-opened, never replaced: its key is
        // random per create (V4) and peers hold it.
        let held = self
            .owner()
            .hold(
                pool,
                OwnedRecord::Profile,
                &stored.profile,
                |key, owner| async move { profile::open_profile(pool, &key, &owner).await },
                || async move {
                    let (lease, key, owner, outcome) =
                        profile::create_profile(pool, fields).await?;
                    if outcome.missed() {
                        tracing::warn!(
                            ?outcome,
                            "own records: new profile not stored at consensus"
                        );
                    }
                    Ok((lease, key, owner))
                },
            )
            .await?;
        *self.profile_lease.lock() = Some(held.lease);
        if !held.created {
            match profile::publish_profile_fields(pool, &held.key, fields).await {
                Ok(outcome) if outcome.missed() => {
                    tracing::warn!(?outcome, "own records: profile fields not all stored");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "own records: profile fields not written"),
            }
        }
        Some((held.key, held.keypair))
    }

    /// Name our prekey record in the profile (subkey 5), now that it holds
    /// its first-contact keys; nothing when it could not be held.
    async fn device_list(&self, pool: &dyn RecordPort, profile_key: &str, list: &[u8]) {
        if list.is_empty() {
            return;
        }
        match profile::publish_device_list(pool, profile_key, list).await {
            Ok(outcome) if outcome.missed() => {
                tracing::warn!(?outcome, "own records: device list not stored at consensus");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "own records: device list not written"),
        }
    }

    async fn friend_list(
        &self,
        pool: &dyn RecordPort,
        stored: &Stored,
    ) -> Option<(String, String)> {
        let held = self
            .owner()
            .hold(
                pool,
                OwnedRecord::FriendList,
                &stored.friend_list,
                |key, owner| async move { friends::open_friend_list(pool, &key, &owner).await },
                || async move {
                    let (lease, key, owner, outcome) = friends::create_friend_list(pool).await?;
                    if outcome.missed() {
                        tracing::warn!(?outcome, "own records: empty friend list not stored");
                    }
                    Ok((lease, key, owner))
                },
            )
            .await?;
        Some((held.key, held.keypair))
    }

    async fn account(&self, pool: &dyn RecordPort, stored: &Stored) {
        let secret = &self.secret;
        let display_name = self.display_name.as_str();
        self.owner()
            .hold(
                pool,
                OwnedRecord::Account,
                &stored.account,
                |key, owner| async move {
                    let encryption_key = rekindle_crypto::DhtRecordKey::derive_account_key(secret);
                    account::AccountRecord::open(pool, &key, &owner, encryption_key)
                        .await
                        .map(|record| record.lease())
                },
                || async move {
                    let encryption_key = rekindle_crypto::DhtRecordKey::derive_account_key(secret);
                    let (record, owner, outcome) =
                        account::AccountRecord::create(pool, encryption_key, display_name, "")
                            .await?;
                    if outcome.missed() {
                        tracing::warn!(?outcome, "own records: account header not stored");
                    }
                    Ok((record.lease(), record.record_key(), owner))
                },
            )
            .await;
    }

    /// Write the keys into the session projection.
    fn project(
        &self,
        keys: &OwnRecordKeys,
        (profile_owner, friend_list_owner): (Option<String>, Option<String>),
    ) {
        let mut guard = self.session.write();
        let Some(session) = guard.as_mut() else {
            return;
        };
        let identity = &mut session.identity;
        if let Some(key) = &keys.profile {
            identity.profile_dht_key.clone_from(key);
        }
        if let Some(key) = &keys.mailbox {
            identity.mailbox_dht_key.clone_from(key);
        }
        if let Some(key) = &keys.friend_list {
            identity.friend_list_dht_key.clone_from(key);
        }
        identity.profile_owner_keypair = profile_owner;
        identity.friend_list_owner_keypair = friend_list_owner;
    }
}

/// Move our profile to a new record under a fresh owner key (a block, so
/// the blocked user's copy of the old key finds nothing), saved by the
/// owned-record rule (`super::owned_record`): the new vault entry, then
/// its key in the database (a crash between the two is repaired at the
/// next unlock, which follows the vault), then the projection; the old
/// record is released and our status is written to the new one. The new
/// record's key.
///
/// # Errors
/// No unlocked identity, or the record could not be created or saved
/// (the new record is released and nothing changes).
pub(crate) async fn rotate_profile(
    ctx: &crate::daemon::dispatch::DaemonContext,
) -> Result<String, String> {
    let transport = ctx
        .transport
        .read()
        .clone()
        .ok_or("transport not started")?;
    let pool = transport.require_records().map_err(|e| e.to_string())?;
    let (owner, display_name) = ctx
        .session
        .read()
        .as_ref()
        .map(|s| {
            (
                s.identity.public_key_hex.clone(),
                s.identity.display_name.clone(),
            )
        })
        .ok_or("no identity is unlocked")?;
    let device_list = super::own_prekeys::device_list_of(ctx, &owner).await?;
    let route_blob = transport.personal_route_blob().unwrap_or_default();
    let status_message = ctx.own_presence.view().status_message;
    let (lease, key, keypair, outcome) = profile::create_profile(
        pool.as_ref(),
        profile::ProfileFields {
            display_name: &display_name,
            status_message: &status_message,
            prekey_records: Some(&device_list),
            route_blob: &route_blob,
        },
    )
    .await
    .map_err(|e| format!("create new profile record: {e}"))?;
    if outcome.missed() {
        tracing::warn!(?outcome, "rotated profile not stored at consensus");
    }
    let saved = Owner {
        vault: &ctx.vault,
        db: &ctx.db,
        public_key: &owner,
    }
    .save(OwnedRecord::Profile, &key, &keypair)
    .await;
    if let Err(e) = saved {
        pool.release(lease).await;
        return Err(e);
    }
    if let Some(session) = ctx.session.write().as_mut() {
        session.identity.profile_dht_key.clone_from(&key);
        session.identity.profile_owner_keypair = Some(keypair);
    }
    // The old profile is no longer ours to write.
    let old = ctx.own_profile_lease.lock().replace(lease);
    if let Some(old) = old {
        pool.release(old).await;
    }
    ctx.own_presence.republish();
    Ok(key)
}

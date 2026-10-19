//! The own-records stage (plan E1.1): the unlocked identity's mailbox,
//! profile, friend list and account, on the record port, on
//! both hosts. Opened when the identity has them, created when it does
//! not (a new identity), so create needs no transport of its own.
//!
//! A new owner keypair goes into the vault before its record key goes into
//! the database, so a stored key always has its keypair. The keys land in
//! the session projection, and the current general route is published to
//! the profile and the mailbox; a route that lands later is the transport's
//! route publisher's.

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_crypto::signal::SignalSessionManager;
use rekindle_db::repo::identity::OwnedRecord;
use rekindle_db::DbHandle;
use rekindle_protocol::dht::{account, friends, mailbox, profile};
use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, RouteClass};
use rekindle_transport::{Session, TransportNode};
use rekindle_vault::typed::owner_keys::{load_owner_keypair, persist_owner_keypair, OwnerKey};
use rekindle_vault::VaultHandle;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

use super::shell_hooks::{OwnRecordKeys, ShellHooks};

/// What the stage needs, owned so it runs in the unlock scope.
pub(crate) struct OwnRecords {
    pub transport: Arc<TransportNode>,
    pub db: Arc<DbHandle>,
    pub vault: VaultHandle,
    pub session: Arc<RwLock<Option<Session>>>,
    pub signal: Arc<SignalSessionManager>,
    pub shell: Option<Arc<dyn ShellHooks>>,
    /// Our presence: its message goes into the profile, and its publisher
    /// writes our status once the profile exists.
    pub presence: super::own_presence::OwnPresence,
    /// Where the profile's session lease is kept, so a rotation (block)
    /// can release it (`DaemonContext.own_profile_lease`).
    pub profile_lease: Arc<parking_lot::Mutex<Option<LeaseId>>>,
    pub public_key: String,
    pub display_name: String,
    pub secret: Zeroizing<[u8; 32]>,
}

/// The records' keys as the identity row and the vault hold them.
#[derive(Default)]
struct Stored {
    profile: Option<String>,
    friend_list: Option<String>,
    account: Option<String>,
    mailbox: Option<String>,
    profile_owner: Option<String>,
    friend_list_owner: Option<String>,
    account_owner: Option<String>,
}

impl OwnRecords {
    /// Run the stage until it is done or `stop` (the unlock ends).
    pub(crate) async fn run(self, stop: CancellationToken) {
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
            stored.profile_owner.clone(),
            stored.friend_list_owner.clone(),
        );

        if stop.is_cancelled() {
            return;
        }
        keys.mailbox = self
            .mailbox(
                pool.as_ref(),
                stored.mailbox.clone(),
                &identity_keypair,
                &route_blob,
            )
            .await;
        if stop.is_cancelled() {
            return;
        }
        if let Some((key, owner)) = self.profile(pool.as_ref(), &stored, &route_blob).await {
            keys.profile = Some(key);
            owners.0 = Some(owner);
        }
        if stop.is_cancelled() {
            return;
        }
        if let Some((key, owner)) = self.friend_list(pool.as_ref(), &stored).await {
            keys.friend_list = Some(key);
            owners.1 = Some(owner);
        }
        if stop.is_cancelled() {
            return;
        }
        keys.account = self.account(pool.as_ref(), &stored).await;
        if stop.is_cancelled() {
            return;
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
        let owner = |record| load_owner_keypair(vault, record).map_err(|e| e.to_string());
        Ok(Stored {
            profile: columns.profile,
            friend_list: columns.friend_list,
            account: columns.account,
            mailbox: columns.mailbox,
            profile_owner: owner(OwnerKey::Profile)?,
            friend_list_owner: owner(OwnerKey::FriendList)?,
            account_owner: owner(OwnerKey::Account)?,
        })
    }

    /// Save a new owner keypair, then its record's key.
    async fn persist(
        &self,
        owner: OwnerKey,
        keypair: &str,
        record: OwnedRecord,
        key: &str,
    ) -> bool {
        {
            let guard = self.vault.lock();
            let Some(vault) = guard.as_ref() else {
                tracing::warn!("own records: the vault closed before a keypair was saved");
                return false;
            };
            if let Err(e) = persist_owner_keypair(vault, owner, keypair) {
                tracing::error!(error = %e, "own records: owner keypair not saved");
                return false;
            }
        }
        let Ok(db) = self.db.current() else {
            return false;
        };
        let (pk, key) = (self.public_key.clone(), key.to_string());
        db.call(move |conn| rekindle_db::repo::identity::set_owned_record(conn, &pk, record, &key))
            .await
            .map_err(|e| tracing::error!(error = %e, "own records: record key not saved"))
            .is_ok()
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
        let bundle = match self.signal.current_bundle() {
            Ok(bundle) => bundle.to_wire(),
            Err(e) => {
                tracing::error!(error = %e, "own records: no prekey bundle");
                return None;
            }
        };
        let status_message = self.presence.view().status_message;
        let fields = profile::ProfileFields {
            display_name: &self.display_name,
            status_message: &status_message,
            prekey_bundle: &bundle,
            route_blob,
        };
        // An existing profile is re-opened, never replaced: its key is
        // random per create (V4) and peers hold it.
        match (&stored.profile, &stored.profile_owner) {
            (Some(key), Some(owner)) => {
                match profile::open_profile(pool, key, owner).await {
                    Ok(lease) => *self.profile_lease.lock() = Some(lease),
                    Err(e) => {
                        tracing::warn!(error = %e, "own records: profile not reopened");
                        return None;
                    }
                }
                match profile::publish_profile_fields(pool, key, fields).await {
                    Ok(outcome) if outcome.missed() => {
                        tracing::warn!(?outcome, "own records: profile fields not all stored");
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "own records: profile fields not written"),
                }
                Some((key.clone(), owner.clone()))
            }
            (Some(key), None) => {
                tracing::error!(key, "own records: the profile has no stored owner keypair");
                None
            }
            (None, _) => {
                let (lease, key, owner, outcome) = profile::create_profile(pool, fields)
                    .await
                    .map_err(|e| tracing::warn!(error = %e, "own records: profile not created"))
                    .ok()?;
                *self.profile_lease.lock() = Some(lease);
                if outcome.missed() {
                    tracing::warn!(?outcome, "own records: new profile not stored at consensus");
                }
                self.persist(OwnerKey::Profile, &owner, OwnedRecord::Profile, &key)
                    .await
                    .then_some((key, owner))
            }
        }
    }

    async fn friend_list(
        &self,
        pool: &dyn RecordPort,
        stored: &Stored,
    ) -> Option<(String, String)> {
        match (&stored.friend_list, &stored.friend_list_owner) {
            (Some(key), Some(owner)) => {
                if let Err(e) = friends::open_friend_list(pool, key, owner).await {
                    tracing::warn!(error = %e, "own records: friend list not reopened");
                    return None;
                }
                Some((key.clone(), owner.clone()))
            }
            (Some(key), None) => {
                tracing::error!(
                    key,
                    "own records: the friend list has no stored owner keypair"
                );
                None
            }
            (None, _) => {
                let (_, key, owner, outcome) = friends::create_friend_list(pool)
                    .await
                    .map_err(|e| tracing::warn!(error = %e, "own records: friend list not created"))
                    .ok()?;
                if outcome.missed() {
                    tracing::warn!(?outcome, "own records: empty friend list not stored");
                }
                self.persist(OwnerKey::FriendList, &owner, OwnedRecord::FriendList, &key)
                    .await
                    .then_some((key, owner))
            }
        }
    }

    async fn account(&self, pool: &dyn RecordPort, stored: &Stored) -> Option<String> {
        let encryption_key = rekindle_crypto::DhtRecordKey::derive_account_key(&self.secret);
        match (&stored.account, &stored.account_owner) {
            (Some(key), Some(owner)) => {
                account::AccountRecord::open(pool, key, owner, encryption_key)
                    .await
                    .map(|record| record.record_key())
                    .map_err(|e| tracing::warn!(error = %e, "own records: account not reopened"))
                    .ok()
            }
            (Some(key), None) => {
                tracing::error!(key, "own records: the account has no stored owner keypair");
                None
            }
            (None, _) => {
                let (record, owner, outcome) =
                    account::AccountRecord::create(pool, encryption_key, &self.display_name, "")
                        .await
                        .map_err(|e| tracing::warn!(error = %e, "own records: account not created"))
                        .ok()?;
                if outcome.missed() {
                    tracing::warn!(?outcome, "own records: account header not stored");
                }
                let key = record.record_key();
                self.persist(OwnerKey::Account, &owner, OwnedRecord::Account, &key)
                    .await
                    .then_some(key)
            }
        }
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
/// the blocked user's copy of the old key finds nothing), written as the
/// stage writes one: the new keypair into the vault, then its key into the
/// database, then the projection; the old record is released and our
/// status is written to the new one. The new record's key.
///
/// # Errors
/// No unlocked identity, or the record could not be created or stored.
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
    let signal = ctx.signal.read().clone().ok_or("no identity is unlocked")?;
    let bundle = signal
        .current_bundle()
        .map_err(|e| format!("prekey bundle: {e}"))?
        .to_wire();
    let route_blob = transport.personal_route_blob().unwrap_or_default();
    let status_message = ctx.own_presence.view().status_message;
    let (lease, key, keypair, outcome) = profile::create_profile(
        pool.as_ref(),
        profile::ProfileFields {
            display_name: &display_name,
            status_message: &status_message,
            prekey_bundle: &bundle,
            route_blob: &route_blob,
        },
    )
    .await
    .map_err(|e| format!("create new profile record: {e}"))?;
    if outcome.missed() {
        tracing::warn!(?outcome, "rotated profile not stored at consensus");
    }
    {
        let guard = ctx.vault.lock();
        let vault = guard.as_ref().ok_or("the vault is locked")?;
        persist_owner_keypair(vault, OwnerKey::Profile, &keypair).map_err(|e| e.to_string())?;
    }
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let (pk, k) = (owner, key.clone());
    db.call(move |conn| {
        rekindle_db::repo::identity::set_owned_record(conn, &pk, OwnedRecord::Profile, &k)
    })
    .await
    .map_err(|e| e.to_string())?;
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

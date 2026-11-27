//! Our prekey record in the own-records stage (plan E2.4): held by the
//! owned-record rule (`super::owned_record`: reopened with its vault
//! entry, repaired after an interrupted write, or created), its session
//! lease kept for the prekey maintenance and the refills; reconciled with
//! our rows and topped up before the profile's subkey 5 names it, and
//! named only once its head is on the network, so a
//! friend never reads an empty record or one without its signed prekey (the stage then lets the
//! maintenance start); and this device's id, minted once.

use std::sync::Arc;

use rekindle_crypto::signal::prekey_record::Publish;
use rekindle_db::repo::identity::OwnedRecord;
use rekindle_protocol::dht::prekeys as record;
use rekindle_records::port::RecordPort;
use rekindle_transport::operations::prekeys::device_list;
use tokio::sync::OwnedMutexGuard;

use super::own_records::{OwnRecords, Stored};

impl OwnRecords {
    /// Hold our prekey record and return the device list naming it, with
    /// the pass's values written after the profile ([`Self::publish_later`]);
    /// the list is empty when the record could not be held (the profile
    /// then names no record, and a friend's first contact waits for the
    /// next unlock). The record's writer guard is taken first and
    /// returned with the later values, held until they are written.
    pub(super) async fn prekeys(
        &self,
        pool: &dyn RecordPort,
        stored: &Stored,
    ) -> (Vec<u8>, Vec<Publish>, Option<OwnedMutexGuard<()>>) {
        let writes = Arc::clone(&self.prekey_writes).lock_owned().await;
        let Some(device_id) = self.device_id(stored).await else {
            return (Vec::new(), Vec::new(), None);
        };
        let held = self
            .owner()
            .hold(
                pool,
                OwnedRecord::Prekeys,
                &stored.prekeys,
                |key, owner| async move { record::open_prekey_record(pool, &key, &owner).await },
                || async move { record::create_prekey_record(pool).await },
            )
            .await;
        let Some(held) = held else {
            return (Vec::new(), Vec::new(), None);
        };
        *self.prekey_lease.lock() = Some(held.lease);
        let key = held.key;
        let (later, head_out) = self.fill_record(pool, &key).await;
        if !head_out {
            // Never named without its signed prekey (live finding L-FR3).
            tracing::warn!(
                "own records: prekey head not on the network; the profile names no record"
            );
            return (Vec::new(), later, Some(writes));
        }
        let list = device_list(&key, &device_id)
            .map_err(|e| tracing::error!(error = %e, "own records: no device list"))
            .unwrap_or_default();
        (list, later, Some(writes))
    }

    /// The unlock's reconciliation (what our rows say the record holds and
    /// it does not), then a maintenance pass (a new identity's first set).
    /// The pass's first-contact part (`split_first_contact`) is written
    /// before the profile names the record; the rest is returned for
    /// [`Self::publish_later`], with whether the head is on the network
    /// ([`head_landed`]): the profile names the record only then, so a
    /// friend never reads a record without its signed prekey (live finding
    /// L-FR3: a head whose first write missed consensus was held for
    /// re-push while the device list already named the record).
    async fn fill_record(&self, pool: &dyn RecordPort, key: &str) -> (Vec<Publish>, bool) {
        use rekindle_crypto::signal::prekey_record::split_first_contact;
        use rekindle_transport::operations::prekeys;
        // Subscribed before any write, so the head's settle is never missed.
        let mut settled = pool.settled();
        match prekeys::reconcile(&self.friends).await {
            Ok(0) => {}
            Ok(republished) => tracing::info!(republished, "prekey record reconciled"),
            Err(e) => tracing::warn!(error = %e, "own records: prekey record not reconciled"),
        }
        let values = match prekeys::plan_and_store(&self.friends).await {
            Ok(values) => values,
            Err(e) => {
                tracing::warn!(error = %e, "own records: prekey maintenance failed");
                return (Vec::new(), head_landed(pool, key, &mut settled).await);
            }
        };
        let (values, later) = split_first_contact(values);
        let mut written = true;
        if let Err(e) = prekeys::publish(&self.friends, values).await {
            // A write that failed outright is not held: the reconcile reads
            // the record back and writes what it lacks once more.
            tracing::warn!(error = %e, "own records: prekeys not published; reconciling");
            written = prekeys::reconcile(&self.friends)
                .await
                .map_err(
                    |e| tracing::warn!(error = %e, "own records: prekey record not reconciled"),
                )
                .is_ok();
        }
        (later, written && head_landed(pool, key, &mut settled).await)
    }

    /// Write the pass's later slots in the unlock scope, in bounded
    /// concurrent batches under the DHT cap (`prekeys::publish`), still
    /// under the stage's writer guard `writes`.
    pub(super) fn publish_later(&self, later: Vec<Publish>, writes: Option<OwnedMutexGuard<()>>) {
        use rekindle_transport::operations::friend::FriendHost as _;
        use rekindle_transport::operations::prekeys;
        if later.is_empty() {
            return;
        }
        let host = self.friends.clone();
        self.friends
            .scope()
            .spawn_or_drop("publish prekeys", async move {
                let _writes = writes;
                if let Err(e) = prekeys::publish(&host, later).await {
                    tracing::warn!(error = %e, "prekey slots not published");
                }
            });
    }

    /// This device's id: the stored one, or 16 random bytes minted now.
    async fn device_id(&self, stored: &Stored) -> Option<String> {
        if let Some(id) = &stored.device_id {
            return Some(id.clone());
        }
        let mut bytes = [0u8; 16];
        self.rng.fill(&mut bytes);
        let (pk, minted) = (self.public_key.clone(), hex::encode(bytes));
        let db = self.db.current().ok()?;
        db.call(move |conn| rekindle_db::repo::identity::ensure_device_id(conn, &pk, &minted))
            .await
            .map_err(|e| tracing::error!(error = %e, "own records: device id not saved"))
            .ok()
    }
}

/// Whether our record's head is on the network: not held for re-push, or
/// held and then settled (`RecordPort::settled`, published on each re-push
/// that lands or is superseded by a newer head of ours). A held write
/// reaches the replicas only when a re-push lands (with `AllowOffline(false)`
/// a set below consensus stores nothing, 0.5.8 `set_value.rs:731-738`,
/// `local_record_store_interface.rs:60-62`), so a reader before that sees
/// no head. `false` only when the pool closed (the session ended).
async fn head_landed(
    pool: &dyn RecordPort,
    key: &str,
    settled: &mut tokio::sync::broadcast::Receiver<rekindle_records::port::Settled>,
) -> bool {
    use rekindle_types::dht_layout::prekeys::HEAD;
    use tokio::sync::broadcast::error::RecvError;
    loop {
        let held = pool
            .held_writes()
            .iter()
            .any(|(record, subkey)| record == key && *subkey == HEAD);
        if !held {
            return true;
        }
        match settled.recv().await {
            Ok(s) if s.record_key == key && s.subkey == HEAD => return true,
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return false,
        }
    }
}

/// The device list the unlocked identity's profile carries, from its
/// identity row (a block rotation writes it into the new profile).
///
/// # Errors
/// No record or device id is stored yet, or the row could not be read.
pub(crate) async fn device_list_of(
    ctx: &crate::daemon::dispatch::DaemonContext,
    owner: &str,
) -> Result<Vec<u8>, String> {
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let pk = owner.to_string();
    let columns = db
        .call(move |conn| rekindle_db::repo::identity::login_row(conn, &pk))
        .await
        .map_err(|e| e.to_string())?
        .map(|(_, columns)| columns)
        .ok_or("the identity database has no identity row")?;
    let (Some(key), Some(device)) = (columns.prekeys, columns.device_id) else {
        return Err("no prekey record is held yet".to_string());
    };
    device_list(&key, &device).map_err(|e| e.to_string())
}

/// Rotate the record and the device id after the user's re-enrolment
/// commit. The replacement is readable before it is named publicly.
pub(crate) async fn replace_for_re_enrol(
    ctx: &crate::daemon::dispatch::DaemonContext,
) -> Result<(), String> {
    use futures::stream::{self, StreamExt as _, TryStreamExt as _};
    use rekindle_crypto::signal::prekey_record::{mint_set, set_values, split_first_contact};
    use rekindle_transport::operations::friend::FriendHost as _;

    let _writes = ctx.own_prekey_writes.lock().await;
    let friends = super::adapters::friends::HostFriends::of(ctx);
    let owner = ctx.require_owner().map_err(|_| "the identity is locked")?;
    let seed = zeroize::Zeroizing::new(ctx.identity_secret().ok_or("the identity is locked")?);
    let node = friends.node().map_err(|e| e.to_string())?;
    let pool = node.require_records().map_err(|e| e.to_string())?;
    let profile = ctx
        .session
        .read()
        .as_ref()
        .map(|session| session.identity.profile_dht_key.clone())
        .filter(|key| !key.is_empty())
        .ok_or("the profile record is not held")?;
    let db = ctx.db.current().map_err(|e| e.to_string())?;
    let identity = rekindle_crypto::Identity::from_secret_bytes(&seed);
    let keys = mint_set(&identity, ctx.rng.as_ref(), ctx.clock.now_ms_i64());
    let (first, later) = split_first_contact(set_values(&keys));
    let device_id = hex::encode(rekindle_utils::random::id_bytes_16_from(ctx.rng.as_ref()));

    let (lease, key, keypair) = record::create_prekey_record(pool.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    let keypair = zeroize::Zeroizing::new(keypair);
    let staged = stream::iter(first)
        .map(|value| {
            let pool = pool.as_ref();
            async move {
                let subkey = value.subkey;
                record::write_once(pool, lease, subkey, value.value)
                    .await
                    .map_err(|e| e.to_string())?
                    .require_stored(subkey)
                    .map_err(|e| e.to_string())
            }
        })
        .buffer_unordered(8)
        .try_collect::<Vec<_>>()
        .await;
    if let Err(error) = staged {
        pool.release(lease).await;
        return Err(error);
    }
    let staged_owner = ctx
        .vault
        .lock()
        .as_ref()
        .ok_or("the vault is locked")
        .and_then(|vault| {
            use zeroize::Zeroize as _;
            let mut entry = rekindle_vault::typed::owner_keys::OwnerEntry {
                record_key: key.clone(),
                keypair: keypair.to_string(),
            };
            let result = rekindle_vault::typed::owner_keys::stage_prekey_owner(vault, &entry);
            entry.keypair.zeroize();
            result.map_err(|error| {
                tracing::warn!(%error, "replacement prekey owner not saved");
                "the replacement prekey owner could not be saved"
            })
        });
    if let Err(error) = staged_owner {
        pool.release(lease).await;
        return Err(error.into());
    }
    let (db_id, db_key) = (device_id.clone(), key.clone());
    let saved = db
        .call(move |conn| {
            rekindle_db::repo::identity::replace_device_prekeys(
                conn, &owner, &db_id, &db_key, &keys,
            )
        })
        .await;
    if let Some(vault) = ctx.vault.lock().as_ref() {
        let committed = saved.is_ok().then_some(key.as_str());
        if let Err(error) = rekindle_vault::typed::owner_keys::finish_prekey_owner(vault, committed)
        {
            tracing::warn!(%error, "prekey owner will be recovered on unlock");
        }
    }
    if let Err(error) = saved {
        pool.release(lease).await;
        return Err(error.to_string());
    }
    let old = ctx.own_prekey_lease.lock().replace(lease);
    if let Some(old) = old {
        pool.release(old).await;
    }
    let list = device_list(&key, &device_id).map_err(|e| e.to_string())?;
    rekindle_protocol::dht::profile::publish_device_list(pool.as_ref(), &profile, &list)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(error) = rekindle_transport::operations::prekeys::publish(&friends, later).await {
        tracing::warn!(%error, "remaining prekeys will be reconciled by maintenance");
    }
    Ok(())
}

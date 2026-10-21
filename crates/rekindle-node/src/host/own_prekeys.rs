//! Our prekey record in the own-records stage (plan E2.4): held by the
//! owned-record rule (`super::owned_record`: reopened with its vault
//! entry, repaired after an interrupted write, or created), its session
//! lease kept for the prekey maintenance and the refills; reconciled with
//! our rows and topped up before the profile's subkey 5 names it, so a
//! friend never reads an empty record (the stage then lets the
//! maintenance start); and this device's id, minted once.

use rekindle_db::repo::identity::OwnedRecord;
use rekindle_protocol::dht::prekeys as record;
use rekindle_records::port::RecordPort;
use rekindle_transport::operations::prekeys::device_list;

use super::own_records::{OwnRecords, Stored};

impl OwnRecords {
    /// Hold our prekey record and return the device list naming it; empty
    /// when it could not be held (the profile then names no record, and a
    /// friend's first contact waits for the next unlock).
    pub(super) async fn prekeys(&self, pool: &dyn RecordPort, stored: &Stored) -> Vec<u8> {
        let Some(device_id) = self.device_id(stored).await else {
            return Vec::new();
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
            return Vec::new();
        };
        *self.prekey_lease.lock() = Some(held.lease);
        let key = held.key;
        self.fill_record().await;
        device_list(&key, &device_id)
            .map_err(|e| tracing::error!(error = %e, "own records: no device list"))
            .unwrap_or_default()
    }

    /// The unlock's reconciliation (what our rows say the record holds and
    /// it does not), then a maintenance pass (a new identity's first set).
    /// The pass's first-contact part (`split_first_contact`) is written
    /// before the profile names the record; the rest follows in the unlock
    /// scope.
    async fn fill_record(&self) {
        use rekindle_crypto::signal::prekey_record::split_first_contact;
        use rekindle_transport::operations::friend::FriendHost as _;
        use rekindle_transport::operations::prekeys;
        match prekeys::reconcile(&self.friends).await {
            Ok(0) => {}
            Ok(republished) => tracing::info!(republished, "prekey record reconciled"),
            Err(e) => tracing::warn!(error = %e, "own records: prekey record not reconciled"),
        }
        let values = match prekeys::plan_and_store(&self.friends).await {
            Ok(values) => values,
            Err(e) => {
                tracing::warn!(error = %e, "own records: prekey maintenance failed");
                return;
            }
        };
        let (values, later) = split_first_contact(values);
        if let Err(e) = prekeys::publish(&self.friends, values).await {
            tracing::warn!(error = %e, "own records: prekeys not published");
        }
        if !later.is_empty() {
            let host = self.friends.clone();
            self.friends
                .scope()
                .spawn_or_drop("publish prekeys", async move {
                    if let Err(e) = prekeys::publish(&host, later).await {
                        tracing::warn!(error = %e, "prekey slots not published");
                    }
                });
        }
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

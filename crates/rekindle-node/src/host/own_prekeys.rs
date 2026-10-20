//! Our prekey record in the own-records stage (plan E2.4): reopened with
//! its stored owner keypair, or created (the keypair into the vault, then
//! its key into the database, D1's order), its session lease kept for the
//! prekey maintenance and the refills; reconciled with our rows and topped
//! up before the profile's subkey 5 names it, so a friend never reads an
//! empty record; and this device's id, minted once.

use rekindle_db::repo::identity::OwnedRecord;
use rekindle_protocol::dht::prekeys as record;
use rekindle_records::port::RecordPort;
use rekindle_transport::operations::prekeys::device_list;
use rekindle_vault::typed::owner_keys::OwnerKey;

use super::own_records::{OwnRecords, Stored};

impl OwnRecords {
    /// Hold our prekey record and return the device list naming it; empty
    /// when it could not be held (the profile then names no record, and a
    /// friend's first contact waits for the next unlock).
    pub(super) async fn prekeys(&self, pool: &dyn RecordPort, stored: &Stored) -> Vec<u8> {
        let Some(device_id) = self.device_id(stored).await else {
            return Vec::new();
        };
        let key = match (&stored.prekeys, &stored.prekeys_owner) {
            (Some(key), Some(owner)) => match record::open_prekey_record(pool, key, owner).await {
                Ok(lease) => {
                    *self.prekey_lease.lock() = Some(lease);
                    key.clone()
                }
                Err(e) => {
                    tracing::warn!(error = %e, "own records: prekey record not reopened");
                    return Vec::new();
                }
            },
            (Some(key), None) => {
                tracing::error!(
                    key,
                    "own records: the prekey record has no stored owner keypair"
                );
                return Vec::new();
            }
            (None, _) => {
                let Ok((lease, key, owner)) = record::create_prekey_record(pool).await.map_err(
                    |e| tracing::warn!(error = %e, "own records: prekey record not created"),
                ) else {
                    return Vec::new();
                };
                if !self
                    .persist(OwnerKey::Prekeys, &owner, OwnedRecord::Prekeys, &key)
                    .await
                {
                    pool.release(lease).await;
                    return Vec::new();
                }
                *self.prekey_lease.lock() = Some(lease);
                key
            }
        };
        self.fill_record().await;
        device_list(&key, &device_id)
            .map_err(|e| tracing::error!(error = %e, "own records: no device list"))
            .unwrap_or_default()
    }

    /// The unlock's reconciliation (what our rows say the record holds and
    /// it does not), then a maintenance pass (a new identity's first set).
    async fn fill_record(&self) {
        use rekindle_transport::operations::prekeys;
        match prekeys::reconcile(&self.friends).await {
            Ok(0) => {}
            Ok(republished) => tracing::info!(republished, "prekey record reconciled"),
            Err(e) => tracing::warn!(error = %e, "own records: prekey record not reconciled"),
        }
        if let Err(e) = prekeys::maintain(&self.friends).await {
            tracing::warn!(error = %e, "own records: prekey maintenance failed");
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

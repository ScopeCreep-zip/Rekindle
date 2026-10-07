//! Service for publishing DHT records (profile, friend list, account, mailbox).
//!
//! Extracted from `commands/auth.rs` to keep DHT publish orchestration in the
//! service layer. The "open-or-create" logic lives in `rekindle-protocol`'s
//! `dht::profile`, `dht::friends`, and `dht::account` modules; this module
//! handles state storage and `SQLite` persistence.

use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use crate::state_helpers::DhtRecordType;
use rekindle_db::Db;

// ── Column mapping for generic persist ──────────────────────────────────────

use rekindle_db::repo::identity::OwnedRecord;

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Persist an owned DHT record's key, and its owner keypair when the
/// record was just created (a reopened record keeps the stored one).
async fn persist_dht_key_to_db(
    pool: &Db,
    public_key: &str,
    dht_key: &str,
    new_keypair: Option<veilid_core::KeyPair>,
    record: OwnedRecord,
) -> Result<(), String> {
    let pk = public_key.to_string();
    let dk = dht_key.to_string();
    let keypair = new_keypair.map(|kp| kp.to_string());
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_owned_record(conn, &pk, record, &dk, keypair.as_deref())
    })
    .await
}

/// Parse an optional keypair string into a `KeyPair`, logging a warning on failure.
fn parse_stored_keypair(keypair_str: Option<&String>, label: &str) -> Option<veilid_core::KeyPair> {
    keypair_str.and_then(|s| {
        s.parse()
            .map_err(|e| {
                tracing::warn!(error = %e, "failed to parse stored {label} owner keypair — will create new record");
                e
            })
            .ok()
    })
}

// ── Publish functions ───────────────────────────────────────────────────────

/// Create or reopen the mailbox DHT record and publish the current route blob.
///
/// The mailbox uses the identity Ed25519 keypair as the DHT record owner,
/// making the record key deterministic and permanent for this identity.
pub async fn publish_mailbox(
    state: &SharedState,
    pool: &Db,
    existing_mailbox_key: Option<&String>,
    route_blob: Option<&[u8]>,
) -> Result<(), String> {
    let public_key = state_helpers::current_owner_key(state)
        .map_err(|_| "identity not set before mailbox publish".to_string())?;
    let secret_bytes = {
        let secret = state.identity_secret.lock();
        *secret.as_ref().ok_or("identity secret not available")?
    };

    // Build a Veilid KeyPair from our Ed25519 identity keys.
    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let pub_bytes = identity.public_key_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    let veilid_keypair = veilid_core::KeyPair::new_from_parts(veilid_pubkey, bare_secret);

    // An existing mailbox is re-opened, never replaced: its key is random
    // per create (V4) and peers hold it. A failure is login's to report.
    let record_pool = state_helpers::record_pool(state)?;
    let mailbox_key = if let Some(existing_key) = existing_mailbox_key {
        rekindle_protocol::dht::mailbox::open_mailbox_writable(
            &record_pool,
            existing_key,
            veilid_keypair,
        )
        .await
        .map_err(|e| format!("reopen mailbox: {e}"))?;
        tracing::info!(key = %existing_key, "reopened existing mailbox");
        existing_key.clone()
    } else {
        rekindle_protocol::dht::mailbox::create_mailbox(&record_pool, veilid_keypair)
            .await
            .map_err(|e| format!("create mailbox: {e}"))?
            .1
    };

    // Write route blob to mailbox subkey 0
    if let Some(blob) = route_blob {
        if !blob.is_empty() {
            let outcome = rekindle_protocol::dht::mailbox::update_mailbox_route(
                &record_pool,
                &mailbox_key,
                blob,
            )
            .await
            .map_err(|e| format!("update mailbox route: {e}"))?;
            if outcome.missed() {
                tracing::warn!(?outcome, "mailbox route blob not stored at consensus");
            }
        }
    }

    state_helpers::store_dht_record(state, &mailbox_key, &DhtRecordType::Mailbox);

    let (pk, mk) = (public_key.clone(), mailbox_key.clone());
    db_call(pool, move |conn| {
        rekindle_db::repo::identity::set_mailbox_key(conn, &pk, &mk)
    })
    .await?;

    tracing::info!(mailbox_key = %mailbox_key, "mailbox published to DHT");
    Ok(())
}

/// Re-open our DHT profile record (or create it when the identity has
/// none) and publish identity data.
///
/// Publishes display name, status message, online status, `PreKeyBundle`, and route
/// blob so that friends can discover our presence and establish encrypted sessions.
pub async fn publish_profile(
    state: &SharedState,
    pool: &Db,
    prekey_bundle_bytes: Vec<u8>,
    existing_dht_key: Option<String>,
    dht_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let id = state_helpers::current_identity(state)
        .map_err(|_| "identity not set before DHT publish".to_string())?;
    let (public_key, display_name, status_message) =
        (id.public_key, id.display_name, id.status_message);
    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
    let record_pool = state_helpers::record_pool(state)?;
    let fields = rekindle_protocol::dht::profile::ProfileFields {
        display_name: &display_name,
        status_message: &status_message,
        prekey_bundle: &prekey_bundle_bytes,
        route_blob: &route_blob,
    };
    let owner_keypair = parse_stored_keypair(dht_owner_keypair_str.as_ref(), "profile");

    // An existing profile is re-opened, never replaced: its key is random
    // per create (V4), and peers know it. A failure, or a key stored
    // without its owner keypair, is login's to report.
    let (lease, profile_key, keypair, new_keypair, outcome) =
        match (existing_dht_key, owner_keypair) {
            (Some(key), Some(keypair)) => {
                let lease = rekindle_protocol::dht::profile::open_profile(
                    &record_pool,
                    &key,
                    keypair.clone(),
                )
                .await
                .map_err(|e| format!("reopen profile record: {e}"))?;
                let outcome = rekindle_protocol::dht::profile::publish_profile_fields(
                    &record_pool,
                    &key,
                    fields,
                )
                .await
                .map_err(|e| format!("publish profile fields: {e}"))?;
                (lease, key, keypair, None, outcome)
            }
            (Some(key), None) => {
                return Err(format!("profile record {key} has no stored owner keypair"));
            }
            (None, _) => {
                let (lease, key, keypair, outcome) =
                    rekindle_protocol::dht::profile::create_profile(&record_pool, fields)
                        .await
                        .map_err(|e| format!("create profile record: {e}"))?;
                (lease, key, keypair.clone(), Some(keypair), outcome)
            }
        };
    match outcome {
        rekindle_protocol::dht::pool::SetOutcome::Superseded(_) => {
            tracing::info!("profile publish: a field was already newer on the network");
        }
        missed if missed.missed() => {
            tracing::warn!(outcome = ?missed, "profile fields not all stored at consensus");
        }
        _ => {}
    }

    let is_new = new_keypair.is_some();
    state_helpers::store_dht_record(state, &profile_key, &DhtRecordType::Profile(keypair, lease));
    // The profile is open: the STATUS publisher writes the current status
    // (login's field publish no longer writes one, plan C7.8c).
    crate::services::presence_service::request_status_publish(state);
    persist_dht_key_to_db(
        pool,
        &public_key,
        &profile_key,
        new_keypair,
        OwnedRecord::Profile,
    )
    .await?;

    tracing::info!(
        profile_key = %profile_key,
        has_route_blob = !route_blob.is_empty(),
        is_new,
        "published profile to DHT"
    );
    Ok(())
}

/// Re-open our DHT friend list record, or create it when the identity has
/// none.
pub async fn publish_friend_list(
    state: &SharedState,
    pool: &Db,
    existing_friend_list_key: Option<String>,
    friend_list_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let public_key = state_helpers::current_owner_key(state)
        .map_err(|_| "identity not set before friend list publish".to_string())?;
    let record_pool = state_helpers::record_pool(state)?;
    let owner_keypair = parse_stored_keypair(friend_list_owner_keypair_str.as_ref(), "friend list");

    let (friend_list_key, keypair, new_keypair) = match (existing_friend_list_key, owner_keypair) {
        (Some(key), Some(keypair)) => {
            rekindle_protocol::dht::friends::open_friend_list(&record_pool, &key, keypair.clone())
                .await
                .map_err(|e| format!("reopen friend list record: {e}"))?;
            (key, keypair, None)
        }
        (Some(key), None) => {
            return Err(format!(
                "friend list record {key} has no stored owner keypair"
            ));
        }
        (None, _) => {
            let (key, keypair, outcome) =
                rekindle_protocol::dht::friends::create_friend_list(&record_pool)
                    .await
                    .map_err(|e| format!("create friend list record: {e}"))?;
            if outcome.missed() {
                tracing::warn!(?outcome, "empty friend list not stored at consensus");
            }
            (key, keypair.clone(), Some(keypair))
        }
    };

    let is_new = new_keypair.is_some();
    state_helpers::store_dht_record(state, &friend_list_key, &DhtRecordType::FriendList(keypair));
    persist_dht_key_to_db(
        pool,
        &public_key,
        &friend_list_key,
        new_keypair,
        OwnedRecord::FriendList,
    )
    .await?;

    tracing::info!(
        friend_list_key = %friend_list_key,
        is_new,
        "published friend list to DHT"
    );
    Ok(())
}

/// Create or reopen the private account DHT record.
///
/// The account record is encrypted with a key derived from the identity's Ed25519
/// secret. It holds pointers to contact list, chat list, and invitation list
/// `DHTShortArray`s.
pub async fn publish_account(
    state: &SharedState,
    pool: &Db,
    existing_account_key: Option<String>,
    account_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let id = state_helpers::current_identity(state)
        .map_err(|_| "identity not set before account publish".to_string())?;
    let (public_key, display_name, status_message) =
        (id.public_key, id.display_name, id.status_message);

    let secret_bytes = state
        .identity_secret
        .lock()
        .ok_or("identity secret not available for account key derivation")?;
    let encryption_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);

    let owner_keypair = parse_stored_keypair(account_owner_keypair_str.as_ref(), "account");
    let record_pool = state_helpers::record_pool(state)?;

    // An existing account record is re-opened, never replaced: its key is
    // random per create (V4), and a replacement would orphan the record
    // peers resolve. A failure, or a key stored without its owner keypair,
    // is login's to report.
    let (account_key, new_keypair) = match (existing_account_key, owner_keypair) {
        (Some(existing_key), Some(keypair)) => {
            let record = rekindle_protocol::dht::account::AccountRecord::open(
                &record_pool,
                &existing_key,
                keypair,
                encryption_key,
            )
            .await
            .map_err(|e| format!("reopen account record: {e}"))?;
            tracing::info!(key = %existing_key, "reusing existing account DHT record");
            state_helpers::store_dht_record(state, &record.record_key(), &DhtRecordType::Account);
            return Ok(());
        }
        (Some(existing_key), None) => {
            return Err(format!(
                "account record {existing_key} has no stored owner keypair"
            ));
        }
        (None, _) => {
            let (record, keypair, outcome) =
                rekindle_protocol::dht::account::AccountRecord::create(
                    &record_pool,
                    encryption_key,
                    &display_name,
                    &status_message,
                )
                .await
                .map_err(|e| format!("create account record: {e}"))?;
            if outcome.missed() {
                tracing::warn!(?outcome, "account header not stored at consensus");
            }
            (record.record_key(), Some(keypair))
        }
    };

    state_helpers::store_dht_record(state, &account_key, &DhtRecordType::Account);

    persist_dht_key_to_db(
        pool,
        &public_key,
        &account_key,
        new_keypair,
        OwnedRecord::Account,
    )
    .await?;

    tracing::info!(account_key = %account_key, "published account record to DHT");
    Ok(())
}

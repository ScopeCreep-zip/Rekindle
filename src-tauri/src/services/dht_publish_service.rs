//! Service for publishing DHT records (profile, friend list, account, mailbox).
//!
//! Extracted from `commands/auth.rs` to keep DHT publish orchestration in the
//! service layer. The "open-or-create" logic lives in `rekindle-protocol`'s
//! `dht::profile`, `dht::friends`, and `dht::account` modules; this module
//! handles state storage and `SQLite` persistence.

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::SharedState;
use crate::state_helpers;
use crate::state_helpers::DhtRecordType;

// ── Column mapping for generic persist ──────────────────────────────────────

/// Pair of column names in the `identity` table for a DHT record key
/// and its owner keypair. Used by [`persist_dht_key_to_db`].
struct DhtKeyColumns {
    key_column: &'static str,
    /// Empty string means no keypair column (mailbox).
    keypair_column: &'static str,
}

const PROFILE_COLUMNS: DhtKeyColumns = DhtKeyColumns {
    key_column: "dht_record_key",
    keypair_column: "dht_owner_keypair",
};

const FRIEND_LIST_COLUMNS: DhtKeyColumns = DhtKeyColumns {
    key_column: "friend_list_dht_key",
    keypair_column: "friend_list_owner_keypair",
};

const ACCOUNT_COLUMNS: DhtKeyColumns = DhtKeyColumns {
    key_column: "account_dht_key",
    keypair_column: "account_owner_keypair",
};

const MAILBOX_COLUMNS: DhtKeyColumns = DhtKeyColumns {
    key_column: "mailbox_dht_key",
    keypair_column: "",
};

// ── Shared helpers ──────────────────────────────────────────────────────────

/// Persist a DHT record key (and optionally its owner keypair) to the identity table.
///
/// Uses `COALESCE` so an existing keypair is preserved when `new_keypair` is `None`
/// (record was reopened, not freshly created).
async fn persist_dht_key_to_db(
    pool: &DbPool,
    public_key: &str,
    dht_key: &str,
    new_keypair: Option<veilid_core::KeyPair>,
    columns: &DhtKeyColumns,
) -> Result<(), String> {
    let pk = public_key.to_string();
    let dk = dht_key.to_string();
    let key_col = columns.key_column;

    if columns.keypair_column.is_empty() {
        db_call(pool, move |conn| {
            conn.execute(
                &format!("UPDATE identity SET {key_col} = ?1 WHERE public_key = ?2"),
                rusqlite::params![dk, pk],
            )?;
            Ok(())
        })
        .await
    } else {
        let kp_col = columns.keypair_column;
        let keypair_str = new_keypair.map(|kp| kp.to_string());
        db_call(pool, move |conn| {
            conn.execute(
                &format!(
                    "UPDATE identity SET {key_col} = ?1, \
                     {kp_col} = COALESCE(?3, {kp_col}) \
                     WHERE public_key = ?2"
                ),
                rusqlite::params![dk, pk, keypair_str],
            )?;
            Ok(())
        })
        .await
    }
}

/// Parse an optional keypair string into a `KeyPair`, logging a warning on failure.
fn parse_stored_keypair(
    keypair_str: Option<&String>,
    label: &str,
) -> Option<veilid_core::KeyPair> {
    keypair_str.and_then(|s| {
        s.parse()
            .map_err(|e| {
                tracing::warn!(error = %e, "failed to parse stored {label} owner keypair — will create new record");
                e
            })
            .ok()
    })
}

/// Create a fresh account DHT record (helper for `publish_account`).
async fn create_fresh_account_record(
    routing_context: &veilid_core::RoutingContext,
    encryption_key: rekindle_crypto::DhtRecordKey,
    display_name: &str,
    status_message: &str,
) -> Result<(String, Option<veilid_core::KeyPair>), String> {
    let (record, kp) = rekindle_protocol::dht::account::AccountRecord::create(
        routing_context,
        encryption_key,
        display_name,
        status_message,
    )
    .await
    .map_err(|e| format!("create account record: {e}"))?;
    Ok((record.record_key(), Some(kp)))
}

// ── Publish functions ───────────────────────────────────────────────────────

/// Create or reopen the mailbox DHT record and publish the current route blob.
///
/// The mailbox uses the identity Ed25519 keypair as the DHT record owner,
/// making the record key deterministic and permanent for this identity.
pub async fn publish_mailbox(
    state: &SharedState,
    pool: &DbPool,
    existing_mailbox_key: Option<&String>,
    route_blob: Option<&[u8]>,
) -> Result<(), String> {
    let public_key = state_helpers::current_owner_key(state)
        .map_err(|_| "identity not set before mailbox publish".to_string())?;
    let secret_bytes = {
        let secret = state.identity_secret.lock();
        *secret.as_ref().ok_or("identity secret not available")?
    };

    let routing_context = state_helpers::require_routing_context(state)?;

    // Build a Veilid KeyPair from our Ed25519 identity keys.
    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let pub_bytes = identity.public_key_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);
    let veilid_keypair = veilid_core::KeyPair::new_from_parts(veilid_pubkey, bare_secret);

    let mailbox_key = if let Some(existing_key) = existing_mailbox_key {
        match rekindle_protocol::dht::mailbox::open_mailbox_writable(
            &routing_context,
            existing_key,
            veilid_keypair.clone(),
        )
        .await
        {
            Ok(()) => {
                tracing::info!(key = %existing_key, "reopened existing mailbox");
                existing_key.clone()
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to reopen mailbox — creating new one");
                rekindle_protocol::dht::mailbox::create_mailbox(&routing_context, veilid_keypair)
                    .await
                    .map_err(|e| format!("create mailbox: {e}"))?
            }
        }
    } else {
        rekindle_protocol::dht::mailbox::create_mailbox(&routing_context, veilid_keypair)
            .await
            .map_err(|e| format!("create mailbox: {e}"))?
    };

    // Write route blob to mailbox subkey 0
    if let Some(blob) = route_blob {
        if !blob.is_empty() {
            rekindle_protocol::dht::mailbox::update_mailbox_route(
                &routing_context,
                &mailbox_key,
                blob,
            )
            .await
            .map_err(|e| format!("update mailbox route: {e}"))?;
        }
    }

    state_helpers::store_dht_record(state, &mailbox_key, &DhtRecordType::Mailbox);

    persist_dht_key_to_db(pool, &public_key, &mailbox_key, None, &MAILBOX_COLUMNS).await?;

    tracing::info!(mailbox_key = %mailbox_key, "mailbox published to DHT");
    Ok(())
}

/// Create or reopen a DHT profile record and publish identity data.
///
/// Publishes display name, status message, online status, `PreKeyBundle`, and route
/// blob so that friends can discover our presence and establish encrypted sessions.
pub async fn publish_profile(
    state: &SharedState,
    pool: &DbPool,
    prekey_bundle_bytes: Option<Vec<u8>>,
    existing_dht_key: Option<String>,
    dht_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let id = state_helpers::current_identity(state)
        .map_err(|_| "identity not set before DHT publish".to_string())?;
    let (public_key, display_name, status_message) =
        (id.public_key, id.display_name, id.status_message);
    let route_blob = state_helpers::our_route_blob(state).unwrap_or_default();
    let routing_context = state_helpers::require_routing_context(state)?;
    let temp_dht = rekindle_protocol::dht::DHTManager::new(routing_context);

    let bundle = prekey_bundle_bytes.as_deref().unwrap_or(&[]);
    let owner_keypair = parse_stored_keypair(dht_owner_keypair_str.as_ref(), "profile");

    let (profile_key, keypair, is_new) =
        rekindle_protocol::dht::profile::open_or_create_profile(
            &temp_dht,
            existing_dht_key.as_deref(),
            owner_keypair,
            &display_name,
            &status_message,
            bundle,
            &route_blob,
        )
        .await
        .map_err(|e| format!("DHT profile publish: {e}"))?;

    state_helpers::store_dht_record(
        state,
        &profile_key,
        &DhtRecordType::Profile(keypair.clone()),
    );

    let new_keypair = if is_new { keypair } else { None };
    persist_dht_key_to_db(pool, &public_key, &profile_key, new_keypair, &PROFILE_COLUMNS).await?;

    tracing::info!(
        profile_key = %profile_key,
        has_prekey_bundle = prekey_bundle_bytes.is_some(),
        has_route_blob = !route_blob.is_empty(),
        is_new,
        "published profile to DHT"
    );
    Ok(())
}

/// Create or reopen a DHT friend list record.
pub async fn publish_friend_list(
    state: &SharedState,
    pool: &DbPool,
    existing_friend_list_key: Option<String>,
    friend_list_owner_keypair_str: Option<String>,
) -> Result<(), String> {
    let public_key = state_helpers::current_owner_key(state)
        .map_err(|_| "identity not set before friend list publish".to_string())?;

    let routing_context = state_helpers::require_routing_context(state)?;
    let temp_dht = rekindle_protocol::dht::DHTManager::new(routing_context);

    let owner_keypair =
        parse_stored_keypair(friend_list_owner_keypair_str.as_ref(), "friend list");

    let (friend_list_key, keypair, is_new) =
        rekindle_protocol::dht::friends::open_or_create_friend_list(
            &temp_dht,
            existing_friend_list_key.as_deref(),
            owner_keypair,
        )
        .await
        .map_err(|e| format!("DHT friend list publish: {e}"))?;

    state_helpers::store_dht_record(
        state,
        &friend_list_key,
        &DhtRecordType::FriendList(keypair.clone()),
    );

    let new_keypair = if is_new { keypair } else { None };
    persist_dht_key_to_db(
        pool,
        &public_key,
        &friend_list_key,
        new_keypair,
        &FRIEND_LIST_COLUMNS,
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
    pool: &DbPool,
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

    let routing_context = state_helpers::require_routing_context(state)?;

    let owner_keypair = parse_stored_keypair(account_owner_keypair_str.as_ref(), "account");

    let (account_key, new_keypair) = if let Some(ref existing_key) = existing_account_key {
        if let Some(keypair) = owner_keypair {
            match rekindle_protocol::dht::account::AccountRecord::open(
                &routing_context,
                existing_key,
                keypair,
                encryption_key,
            )
            .await
            {
                Ok(record) => {
                    tracing::info!(key = %existing_key, "reusing existing account DHT record");
                    state_helpers::store_dht_record(
                        state,
                        &record.record_key(),
                        &DhtRecordType::Account,
                    );
                    state_helpers::track_open_records(state, &record.all_record_keys());
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(
                        key = %existing_key, error = %e,
                        "failed to open existing account record — creating new one"
                    );
                    let enc_key =
                        rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
                    create_fresh_account_record(
                        &routing_context,
                        enc_key,
                        &display_name,
                        &status_message,
                    )
                    .await?
                }
            }
        } else {
            tracing::warn!("no account owner keypair — creating new account record");
            let enc_key = rekindle_crypto::DhtRecordKey::derive_account_key(&secret_bytes);
            create_fresh_account_record(&routing_context, enc_key, &display_name, &status_message)
                .await?
        }
    } else {
        create_fresh_account_record(
            &routing_context,
            encryption_key,
            &display_name,
            &status_message,
        )
        .await?
    };

    state_helpers::store_dht_record(state, &account_key, &DhtRecordType::Account);

    persist_dht_key_to_db(
        pool,
        &public_key,
        &account_key,
        new_keypair,
        &ACCOUNT_COLUMNS,
    )
    .await?;

    tracing::info!(account_key = %account_key, "published account record to DHT");
    Ok(())
}

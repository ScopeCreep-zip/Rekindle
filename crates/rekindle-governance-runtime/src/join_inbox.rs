//! The invite-scoped record used for durable join discovery (ADR 0011).

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::{dht_layout::join_inbox, invite::JoinInboxCapability};
use zeroize::Zeroizing;

use crate::{GovernanceRuntimeDeps, GovernanceRuntimeError};

/// The local credentials for the inviter's watch.
#[derive(Clone)]
pub struct MintedJoinInbox {
    pub record_key: String,
    pub owner_keypair: Zeroizing<String>,
}

impl std::fmt::Debug for MintedJoinInbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MintedJoinInbox")
            .field("record_key", &self.record_key)
            .finish_non_exhaustive()
    }
}

/// Publish the owner marker before handing out the write capability.
///
/// # Errors
/// An invalid slot count or a failed record creation or publication.
pub(crate) async fn mint<D: GovernanceRuntimeDeps>(
    deps: &D,
    max_uses: u32,
) -> Result<(JoinInboxCapability, MintedJoinInbox), GovernanceRuntimeError> {
    let slots = u8::try_from(max_uses)
        .map_err(|_| GovernanceRuntimeError::OutOfBounds { field: "max_uses" })?;
    let mut seed = Zeroizing::new([0; 32]);
    deps.rng().fill(seed.as_mut_slice());
    let signing = SigningKey::from_bytes(&seed);
    let public = signing.verifying_key().to_bytes();
    let schema = rekindle_records::schema::join_inbox(&public, slots)
        .ok_or(GovernanceRuntimeError::OutOfBounds { field: "max_uses" })?;
    let pool = deps.record_port()?;
    let (lease, record_key, owner) = pool
        .create(&schema, None)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("create join inbox: {e}")))?;
    let owner_keypair = Zeroizing::new(owner);
    let opened = pool
        .set(
            lease,
            join_inbox::OWNER_SUBKEY,
            join_inbox::OPENED.to_vec(),
            None,
        )
        .await
        .and_then(|outcome| outcome.require_stored(join_inbox::OWNER_SUBKEY));
    pool.release(lease).await;
    opened.map_err(|e| GovernanceRuntimeError::Adapter(format!("publish join inbox: {e}")))?;
    Ok((
        JoinInboxCapability {
            join_inbox: record_key.clone(),
            join_inbox_writer: Zeroizing::new(deps.format_writer_keypair(public, *seed)),
            join_inbox_slots: slots,
        },
        MintedJoinInbox {
            record_key,
            owner_keypair,
        },
    ))
}

/// The result of validating a locator against the signed member record
/// and the same governance admission used by the bootstrap exchange.
pub enum Receipt {
    Accepted(crate::BootstrapJoiner),
    Unreadable,
    Pending,
}

/// Check a locator without trusting it as a membership claim. A missing
/// op or dependency remains pending; permanent decode, owner and invite
/// mismatches are unreadable.
pub async fn receive<D: GovernanceRuntimeDeps>(
    deps: &D,
    community: &str,
    record: &str,
    invite_id: &[u8; 16],
    code_hash: &str,
    sealed: &[u8],
) -> Receipt {
    use rekindle_codec::{governance_op::decode_op, join_locator::JoinLocator};
    use rekindle_types::governance::GovernanceEntry;

    let Some(secret) = deps.identity_secret() else {
        return Receipt::Pending;
    };
    if crate::partial::is_partial(deps, community).await {
        return Receipt::Pending;
    }
    let recipient = rekindle_secrets::derive::derive_community_pseudonym(&secret, community);
    let Ok(locator) = JoinLocator::open(sealed, &recipient, invite_id) else {
        return Receipt::Unreadable;
    };
    let reference = locator.joined_op;
    let Ok(Some(held)) =
        crate::op_log::fetch_held(deps, &reference, &crate::op_walk::WalkLogCache::default()).await
    else {
        return Receipt::Pending;
    };
    let Ok(decoded) = decode_op(&held.encoded) else {
        return Receipt::Unreadable;
    };
    let GovernanceEntry::MemberJoined {
        member_record,
        member_record_owner,
        invite_proof: Some(proof),
        ..
    } = decoded.op.body
    else {
        return Receipt::Unreadable;
    };
    if proof.code_hash != code_hash || member_record != reference.author_record {
        return Receipt::Unreadable;
    }
    let Ok(pool) = deps.record_port() else {
        return Receipt::Pending;
    };
    let Ok(lease) = pool
        .acquire(
            &member_record,
            None,
            rekindle_records::port::OpenIntent::Check,
        )
        .await
    else {
        return Receipt::Pending;
    };
    let owner = pool.owner_of(lease);
    pool.release(lease).await;
    match owner {
        Ok(owner) if owner == member_record_owner => {}
        Ok(_) => return Receipt::Unreadable,
        Err(_) => return Receipt::Pending,
    }
    let encoded = held.encoded.clone();
    match crate::bootstrap::admit_joiner(deps, community, &reference, held).await {
        Ok(()) => {}
        Err(GovernanceRuntimeError::JoinNotAdmitted(_)) => return Receipt::Unreadable,
        Err(_) => return Receipt::Pending,
    }
    let route = crate::member_routes::read_member_route(
        &*pool,
        community,
        &member_record,
        &reference.author,
        crate::member_routes::RoutePin::Joined(&member_record_owner),
    )
    .await
    .ok()
    .flatten()
    .unwrap_or_default();
    tracing::debug!(community, inbox = record, "join locator admitted");
    Receipt::Accepted(crate::BootstrapJoiner {
        pseudonym: hex::encode(reference.author.0),
        member_joined: reference,
        member_joined_op: encoded,
        route,
    })
}

//! Origin (plan E3.2, ADR 0011): create a community. The anchor `A`
//! (DFLT(1)) holds the `GenesisCert`, signed by `A`'s owner `K_A` (written
//! once, then released and zeroized) and by the creator pseudonym; the
//! creator's member record `M(p)` (DFLT(8)) is owned by a random `K_M(p)`
//! kept in the vault; the genesis ops are the creator's ops #0…n. The
//! registry is still created until E3.4 deletes it; `#general` has no
//! record (its posts ride `M`/4, plan E3.3). The community id is `A`'s
//! record key.

use rekindle_codec::governance_op::genesis;
use rekindle_crypto::keypair_text;
use rekindle_governance::genesis::genesis_signing_bytes;
use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::keys::{MediaEncryptionKey, SlotSeed};
use rekindle_types::dht_layout::community_anchor;
use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions;
use rekindle_types::presence::MemberPresence;
use zeroize::Zeroizing;

use crate::deps::{
    CommunityInsert, DhtRecordInfo, DiscoveredMember, GovernanceRuntimeDeps, MekSnapshot,
};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log;
use rekindle_records::lease::CommunityLeases;

use crate::origin_reserve::{
    reserve_community, verified_creators, CoCreatorAcceptance, ReservedCommunity,
};

const CREATOR_SLOT: u32 = 0;
/// Owner role gets the all-ones RoleId so the on-the-wire role tag is
/// stable across community installs (architecture §Failure 4).
const OWNER_ROLE_ID: RoleId = RoleId([0xFFu8; 16]);
/// `@everyone`: the all-zero RoleId.
const EVERYONE_ROLE_ID: RoleId = RoleId([0u8; 16]);

/// Generate a fresh random 16-byte channel id from the host's generator.
fn random_channel_id(rng: &dyn rekindle_utils::RandomSource) -> ChannelId {
    ChannelId(rekindle_utils::random::id_bytes_16_from(rng))
}

/// The number of genesis ops, the creator's ops #0…4.
const GENESIS_OPS: usize = 5;

/// The seq of the creator's last genesis op (`ChannelCreated`): the base a
/// join cites while no checkpoint exists (plan E3.2 S7c.0,
/// [`join_deps`](crate::join_deps::join_deps)).
pub const GENESIS_LAST_SEQ: u64 = 4;

/// The genesis ops' bodies, the creator's ops #0…n in this order. The
/// admission mode is not among them: `GenesisCert.policy` pins it (plan
/// E3.2 decision (b)).
pub(crate) fn genesis_entries(
    name: &str,
    my_pseudo: &PseudonymKey,
    channel_id: ChannelId,
) -> [GovernanceEntry; GENESIS_OPS] {
    [
        GovernanceEntry::CommunityMeta {
            name: Some(name.to_string()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
        GovernanceEntry::RoleDefinition {
            role_id: EVERYONE_ROLE_ID,
            name: "@everyone".into(),
            permissions: permissions::DEFAULT_EVERYONE,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
        GovernanceEntry::RoleDefinition {
            role_id: OWNER_ROLE_ID,
            name: "Owner".into(),
            permissions: permissions::ADMINISTRATOR,
            position: u32::MAX,
            color: 0xC1_7C_17,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
        },
        GovernanceEntry::RoleAssignment {
            target: my_pseudo.clone(),
            role_id: OWNER_ROLE_ID,
        },
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: "general".into(),
            channel_type: "text".into(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
        },
    ]
}

/// How long a create may take, from the first record to the creator's
/// beat. Veilid bounds a set's fanout (`set_value_timeout_ms`, 10 s,
/// 0.5.8 `veilid_config.rs:1010`) but not its admission: the gate waits on
/// the set concurrency and upload budgets with no deadline
/// (`storage_manager/gated_operation.rs:460-465`), and with no deadline of
/// ours a create held there never resolved (finding C-COM1). Two of the
/// record pool's retry budgets (30 s each, `rekindle-record-pool`
/// `RETRY_BUDGET`): room for the create's serial writes at a full fanout
/// each, and the user is never left waiting on an unbounded review. At the
/// deadline every record is released and an installed community discarded:
/// the anchor's one-time key is gone, so the create cannot resume later.
pub const CREATE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

/// The records origin creates; every lease is released on a failure.
pub(crate) struct OriginRecords {
    pub(crate) anchor: DhtRecordInfo,
    pub(crate) member: DhtRecordInfo,
    pub(crate) registry: DhtRecordInfo,
}

/// Create a new community (plan E3.2, ADR 0011) with no co-creator: the
/// anchor `A` with its `GenesisCert`, the creator's member record `M(p)`,
/// the genesis ops, and (until E3.4 deletes it) the registry.
/// Returns `A`'s record key, the community id.
pub async fn create_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    name: &str,
    admission: AdmissionMode,
) -> Result<String, GovernanceRuntimeError> {
    let reserved = reserve_community(deps).await?;
    finish_community(deps, reserved, name, admission, &[]).await
}

/// Write the genesis of a reserved community within [`CREATE_DEADLINE`]:
/// the cert lists `accepted` as further creators (each signature
/// verified, the set checked by [`check_creator_set`]), then the genesis
/// ops, the registry row, the first MEK, the install and the records.
/// On a failure every record is released and an installed community
/// discarded.
///
/// # Errors
/// An acceptance does not verify, the creator set is malformed, a write
/// failed, or the deadline passed.
pub async fn finish_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    reserved: ReservedCommunity,
    name: &str,
    admission: AdmissionMode,
    accepted: &[CoCreatorAcceptance],
) -> Result<String, GovernanceRuntimeError> {
    finish_with(deps, reserved, name, admission, accepted, None).await
}

/// Create a successor of `predecessor` (ADR 0011 "Successor community",
/// plan E3.2 S7s.4) with no co-creator: a new community whose
/// `GenesisCert.predecessor` names `predecessor`, the far link readers
/// check before they offer it.
///
/// # Errors
/// As [`create_community`].
pub async fn create_successor_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    name: &str,
    admission: AdmissionMode,
    predecessor: &str,
) -> Result<String, GovernanceRuntimeError> {
    let reserved = reserve_community(deps).await?;
    let predecessor = Some(predecessor.to_owned());
    finish_with(deps, reserved, name, admission, &[], predecessor).await
}

/// Write the genesis of a reserved transfer successor of `predecessor`
/// (plan E3.2 S7s.4): [`finish_community`] whose cert names `predecessor`
/// and lists `accepted`, the linked co-creators' new pseudonyms.
///
/// # Errors
/// As [`finish_community`].
pub async fn finish_successor_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    reserved: ReservedCommunity,
    name: &str,
    admission: AdmissionMode,
    accepted: &[CoCreatorAcceptance],
    predecessor: &str,
) -> Result<String, GovernanceRuntimeError> {
    let predecessor = Some(predecessor.to_owned());
    finish_with(deps, reserved, name, admission, accepted, predecessor).await
}

/// [`finish_community`] with the cert's `predecessor`.
async fn finish_with<D: GovernanceRuntimeDeps>(
    deps: &D,
    reserved: ReservedCommunity,
    name: &str,
    admission: AdmissionMode,
    accepted: &[CoCreatorAcceptance],
    predecessor: Option<String>,
) -> Result<String, GovernanceRuntimeError> {
    let deadline = tokio::time::Instant::now() + CREATE_DEADLINE;
    let ReservedCommunity { slot_seed, records } = reserved;
    let anchor_key = records.anchor.record_key.clone();
    let leases = [
        records.anchor.lease,
        records.member.lease,
        records.registry.lease,
    ];
    let mut installed = false;
    let seeded = match verified_creators(&anchor_key, accepted) {
        Ok(further) => tokio::time::timeout_at(
            deadline,
            seed_community(
                deps,
                Genesis {
                    name,
                    admission,
                    further,
                    predecessor,
                },
                &slot_seed,
                records,
                &mut installed,
            ),
        )
        .await
        .unwrap_or_else(|_| Err(timed_out())),
        Err(e) => Err(e),
    };
    match seeded {
        Ok(anchor) => Ok(anchor),
        Err(e) => {
            for lease in leases {
                deps.release_record(lease).await;
            }
            if installed {
                deps.discard_created_community(&anchor_key).await;
            }
            Err(e)
        }
    }
}

/// What the cert says beyond the records: the name, the admission mode
/// the further creators and the community this one succeeds.
struct Genesis<'a> {
    name: &'a str,
    admission: AdmissionMode,
    further: Vec<PseudonymKey>,
    predecessor: Option<String>,
}

/// The deadline error, naming the bound in seconds.
pub(crate) fn timed_out() -> GovernanceRuntimeError {
    GovernanceRuntimeError::CreateTimedOut(CREATE_DEADLINE.as_secs())
}

/// Create `A`, `M(p)` and the registry in that order, pushing each into
/// `created` as it exists (the caller releases them on a failure).
pub(crate) async fn create_records<D: GovernanceRuntimeDeps>(
    deps: &D,
    member_pubkeys: &[[u8; 32]],
    created: &mut Vec<DhtRecordInfo>,
) -> Result<(), GovernanceRuntimeError> {
    created.push(deps.create_record(&schema::community_anchor()).await?);
    created.push(deps.create_record(&schema::member_record()).await?);
    created.push(deps.create_smpl_record(member_pubkeys).await?);
    Ok(())
}

/// Parse a created record's owner keypair text into its signing key.
fn owner_signing_key(
    info: &DhtRecordInfo,
    what: &str,
) -> Result<(Zeroizing<String>, SigningKey), GovernanceRuntimeError> {
    let text = info.owner_keypair.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!("origin: the {what} has no owner keypair"))
    })?;
    let (_, secret) = keypair_text::parse_keypair_text(&text)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("{what} owner: {e}")))?;
    let secret = Zeroizing::new(secret);
    Ok((text, SigningKey::from_bytes(&secret)))
}

/// Sign and write the `GenesisCert` to `A`/0 with `K_A`, then release `A`
/// and drop `K_A` (zeroized; never persisted). Returns the cert and its
/// stored bytes.
async fn write_genesis<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &DhtRecordInfo,
    cert: GenesisCert,
    pseudonym: &SigningKey,
) -> Result<(GenesisCert, Vec<u8>), GovernanceRuntimeError> {
    let (k_a_text, k_a) = owner_signing_key(anchor, "anchor")?;
    let anchor_owner = k_a.verifying_key().to_bytes();
    let unsigned = genesis::genesis_unsigned_bytes(&cert, &anchor_owner)?;
    let message = genesis_signing_bytes(&anchor.record_key, &unsigned);
    let signed = genesis::SignedGenesis {
        anchor_sig: k_a.sign(&message).to_bytes(),
        creator_sig: pseudonym.sign(&message).to_bytes(),
        cert,
        anchor_owner,
    };
    drop(k_a);
    let bytes = genesis::encode_genesis(&signed)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("genesis cert: {e}")))?;
    let outcome = deps
        .set_dht_value(
            anchor.lease,
            community_anchor::GENESIS_CERT,
            bytes.clone(),
            Some(k_a_text),
        )
        .await;
    deps.release_record(anchor.lease).await;
    if let Some(stale) = outcome? {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }
    Ok((signed.cert, bytes))
}

/// Origin after the records exist: the cert, the genesis ops, the
/// creator's registry row, the first MEK, the local install (`installed`
/// is set just before it), and handing the records to the host.
async fn seed_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    genesis: Genesis<'_>,
    slot_seed: &SlotSeed,
    records: OriginRecords,
    installed: &mut bool,
) -> Result<String, GovernanceRuntimeError> {
    let Genesis {
        name,
        admission,
        further,
        predecessor,
    } = genesis;
    let OriginRecords {
        anchor,
        member,
        registry: reg_record,
    } = records;
    let anchor_key = anchor.record_key.clone();
    let reg_key = reg_record.record_key.clone();

    // The creator pseudonym from the master secret and the anchor.
    let master_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, &anchor_key);
    let my_pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    let my_pseudo_hex = hex::encode(my_pseudo_bytes);
    let my_pseudo = PseudonymKey(my_pseudo_bytes);

    // M(p)'s owner K_M(p): the public half goes in the cert, the keypair
    // to the vault through the insert (before the community row, D1).
    let (k_m_text, k_m) = owner_signing_key(&member, "member record")?;
    let member_owner = keypair_text::public_key_text(&k_m.verifying_key().to_bytes());
    drop(k_m);
    // This device's first op key signs the genesis ops; the cert carries
    // `p`'s endorsement of it (ADR 0022 rule 3, plan S7d-keys.5).
    let device = crate::device_signer::device_op_key(deps, &anchor_key, true)?;
    let creator_device = crate::device_signer::first_endorsement(
        &pseudonym_signing,
        &anchor_key,
        &device,
        &member.record_key,
        &member_owner,
    );
    let member_key = member.record_key.clone();
    deps.release_record(member.lease).await;
    let writer = k_m_text.clone();
    let hpke_secret = crate::member_card::new_hpke_secret(deps.rng().as_ref());

    let cert = GenesisCert {
        name: name.to_string(),
        policy: admission,
        initial_roles: vec![EVERYONE_ROLE_ID, OWNER_ROLE_ID],
        creator_pseudonym: my_pseudo.clone(),
        creator_member_record: member_key.clone(),
        creator_member_record_owner: member_owner,
        created_at: deps.clock().now_secs(),
        additional_creators: further,
        predecessor,
        creator_device: Some(creator_device),
    };
    rekindle_governance::genesis::check_creator_set(&cert).map_err(|e| {
        GovernanceRuntimeError::Crypto(format!("origin: the creator set is refused: {e:?}"))
    })?;
    let (cert, cert_bytes) = write_genesis(deps, &anchor, cert, &pseudonym_signing).await?;

    // The genesis ops: the creator's ops #0…n.
    let channel_id = random_channel_id(deps.rng().as_ref());
    let mut set = op_log::VerifiedSet::default();
    let mut genesis_ops = Vec::new();
    for body in genesis_entries(name, &my_pseudo, channel_id) {
        let (op, encoded) = op_log::build_op(
            &anchor_key,
            &cert,
            &GovernanceState::default(),
            &set,
            crate::device_signer::OpSigner {
                author: my_pseudo.clone(),
                device: &device,
            },
            body,
            op_log::Deps::Frontier,
            cert.created_at,
        )?;
        genesis_ops.push(op_log::held(
            &op,
            member_key.clone(),
            encoded,
            op_log::OpFate::Applied,
        ));
        set.ops.push(op);
    }
    let gov_state = merge::merge(&cert, &set.ops);

    let creator_slot_kp = derive::derive_slot_keypair(&slot_seed.0, CREATOR_SLOT).map_err(|e| {
        GovernanceRuntimeError::Crypto(format!("creator slot keypair derivation failed: {e}"))
    })?;
    let creator_writer = deps.format_writer_keypair(
        creator_slot_kp.verifying_key().to_bytes(),
        creator_slot_kp.to_bytes(),
    );

    // The creator's MemberPresence in registry slot 0, signed so peers can
    // verify the creator authored it (until E3.4 replaces the registry).
    let mut presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        // No name (plan E3.4 S6): the identity's name never reaches a
        // community; readers show the short pseudonym fingerprint.
        display_name: None,
        status: deps.identity_status().as_wire_str().into(),
        route_blob: deps.our_route_blob(),
        last_heartbeat: deps.clock().now_secs(),
        ..Default::default()
    };
    let presence_sig = derive::sign_with_pseudonym(&pseudonym_signing, &presence.signing_bytes());
    presence.signature = presence_sig.to_vec();
    let presence_bytes = serde_json::to_vec(&presence).map_err(|e| {
        GovernanceRuntimeError::Encoding(format!("presence serialization failed: {e}"))
    })?;
    let reg_outcome = deps
        .set_dht_value(
            reg_record.lease,
            CREATOR_SLOT,
            presence_bytes,
            Some(Zeroizing::new(creator_writer.clone())),
        )
        .await?;
    if let Some(stale) = reg_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    let mek = MediaEncryptionKey::generate_with(
        rekindle_governance::state::GENESIS_MEK_GENERATION,
        deps.rng().as_ref(),
    );
    let mek_snapshot = MekSnapshot {
        generation: mek.generation(),
        key_bytes: *mek.as_bytes(),
    };
    *installed = true;
    deps.insert_community_mek(&anchor_key, mek_snapshot.clone());

    let insert = CommunityInsert {
        id: anchor_key.clone(),
        name: name.to_string(),
        channel_id_hex: hex::encode(channel_id.0),
        registry_key: reg_key.clone(),
        registry_owner_keypair: reg_record.owner_keypair,
        slot_seed_hex: hex::encode(slot_seed.0),
        slot_keypair: creator_writer,
        my_pseudonym_hex: my_pseudo_hex.clone(),
        mek: mek_snapshot,
        governance_state: gov_state,
        genesis_cert: cert_bytes,
        member_record: member_key.clone(),
        member_record_owner_keypair: k_m_text,
        member_hpke_secret: hpke_secret.clone(),
    };
    // The installed genesis state and the genesis ops it merges land as
    // one step under the rebuild lock (D15: the state is the merge of the
    // ops held), the ops after the row they reference: a convergence check
    // reading between them would take the installed state for a lag and
    // rebuild it from the ops stored so far, dropping the rest.
    {
        let lock = deps.rebuild_lock(&anchor_key);
        let _installing = lock.lock().await;
        deps.insert_community(insert);
        // Every genesis op is ours, signed above: stored `signed_here`
        // (plan E3.2 S7d-keys.3), the unlock barrier's local head (.8).
        for op in &genesis_ops {
            deps.op_store()
                .store_signed_governance_op(&anchor_key, op)
                .await?;
        }
    }

    // M(p) in §4.4's order: the card first (the record is openable), the
    // genesis ops onto the governance-op stream, which appends them to M/2
    // and writes M/3, then the beat.
    let port = deps.record_port()?;
    let own = crate::member_card::OwnRecord {
        community_id: &anchor_key,
        record: &member_key,
        writer: &writer,
        pseudonym: &pseudonym_signing,
    };
    let profile = crate::member_card::OwnProfile {
        status: deps.identity_status(),
        beat_seq: deps
            .next_beat_seq(&anchor_key)
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("origin: beat seq: {e}")))?,
        now_ms: deps.clock().now_ms(),
        route: deps.our_route_blob(),
    };
    let publish_ops = async {
        for op in genesis_ops {
            // The row first, then the gossip (lead ruling 2026-11-26, item 2).
            let publication = deps
                .publish_governance_op(
                    &anchor_key,
                    op.encoded.clone(),
                    crate::deps::OpLanding::FirstAttempt,
                )
                .await
                .map_err(|e| e.to_string())?;
            deps.gossip_governance_op(&anchor_key, &op.encoded);
            publication.landed().await.map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    crate::member_card::publish_own_record(&*port, &own, &profile, &hpke_secret, publish_ops)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("origin: {e}")))?;

    deps.member_record_published(&anchor_key, own.record, &profile.route);
    deps.ensure_files_cache_open(&anchor_key);
    deps.persist_discovered_registry_members(
        &anchor_key,
        vec![DiscoveredMember {
            segment_index: 0,
            slot_index: CREATOR_SLOT,
            presence: presence.clone(),
        }],
    );

    deps.community_records_ready(
        &anchor_key,
        CommunityLeases {
            registry: Some(reg_record.lease),
            ..CommunityLeases::default()
        },
    )
    .await;

    deps.emit_event(GovernanceRuntimeEvent::CommunityCreated {
        community_id: anchor_key.clone(),
        name: name.to_string(),
    });

    Ok(anchor_key)
}

#[cfg(test)]
#[path = "origin_tests.rs"]
mod tests;

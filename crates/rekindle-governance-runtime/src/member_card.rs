//! Our member record's card (`M`/0) and beat (`M`/1), and the reader of
//! another member's (plan E3.2 "Join", "Origin rewritten" and "Records";
//! `evidence/slot-integrity.md` §4.4 step 4).
//!
//! **Write order.** A record is openable by others only once subkey 0
//! exists (V7: a remote open inspects subkey 0 for the descriptor), so the
//! card is written first, before op #0 (`M`/2) and the frontier (`M`/3);
//! the beat comes last. [`publish_own_record`] is that order, for the join
//! and for origin.
//!
//! **Reads.** A value read from `M(p)` is accepted only if the record's
//! descriptor owner (`owner_of`) equals the `memberRecordOwner` from `p`'s
//! `MemberJoined` and the value's inner signature verifies under `p`
//! (`rekindle_codec::member_record`).
//!
//! The beat written here carries no `extras`: presence never waits for the
//! community key (§4.4 step 6), and E3.4 builds the beat engine that
//! rewrites it.

use std::future::Future;

use rekindle_codec::member_record::{
    beat::decode_presence_beat, card::decode_member_card, MemberCard, PresenceBeat, RecordScope,
    SignedMemberCard,
};
use rekindle_records::port::{RecordPort, SetOutcome};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionStatus;
use rekindle_utils::random::RandomSource;
use zeroize::Zeroizing;

/// A beat's lifetime: E3.4 writes it every 60 s with
/// `expiresAt = now + 150 s` (the 2.5x rule the friend TTL uses).
pub const BEAT_LIFETIME_MS: u64 = 150_000;

/// A fresh X25519 secret for our card's `hpkeKey` in one community.
#[must_use]
pub fn new_hpke_secret(rng: &dyn RandomSource) -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0u8; 32]);
    rng.fill(secret.as_mut());
    secret
}

/// Our member record and the keys that write and sign it.
pub struct OwnRecord<'a> {
    pub community_id: &'a str,
    pub record: &'a str,
    /// `K_M(p)`'s keypair text.
    pub writer: &'a str,
    pub pseudonym: &'a SigningKey,
}

impl OwnRecord<'_> {
    fn scope(&self) -> RecordScope<'_> {
        RecordScope {
            community_id: self.community_id,
            member_record: self.record,
        }
    }
}

/// What our first card and beat say.
pub struct OwnProfile {
    pub display_name: String,
    pub status: SessionStatus,
    /// Now, on the host's clock (ms).
    pub now_ms: u64,
}

/// Our card: the identity's display name and the HPKE key, cross-signed.
fn own_card(profile: &OwnProfile, hpke_secret: &[u8; 32]) -> MemberCard {
    MemberCard {
        display_name: Some(profile.display_name.clone()),
        bio: None,
        avatar_ref: None,
        banner_ref: None,
        pronouns: None,
        theme_color: None,
        badges: Vec::new(),
        custom_status: None,
        game: None,
        hpke_key: rekindle_secrets::hpke::public_key(hpke_secret),
    }
}

async fn write_subkey(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    subkey: u32,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let lease = port
        .acquire(own.record, Some(own.writer))
        .await
        .map_err(|e| format!("open member record: {e}"))?;
    let written = port.set(lease, subkey, bytes, Some(own.writer)).await;
    port.release(lease).await;
    match written.map_err(|e| format!("write member record subkey {subkey}: {e}"))? {
        SetOutcome::Landed => Ok(()),
        other => Err(format!(
            "member record subkey {subkey} not stored: {other:?}"
        )),
    }
}

/// Write our card (`M`/0).
///
/// # Errors
/// The card does not encode, or the write did not land.
pub async fn write_card(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    profile: &OwnProfile,
    hpke_secret: &[u8; 32],
) -> Result<(), String> {
    let bytes = own_card(profile, hpke_secret)
        .encode(own.scope(), own.pseudonym)
        .map_err(|e| format!("member card: {e}"))?;
    write_subkey(port, own, member_record::CARD, bytes).await
}

/// Write our first beat (`M`/1), without extras.
///
/// # Errors
/// The beat does not encode, or the write did not land.
pub async fn write_beat(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    profile: &OwnProfile,
) -> Result<(), String> {
    let beat = PresenceBeat {
        seq: 0,
        status: profile.status,
        voice_channel: None,
        expires_at: profile.now_ms.saturating_add(BEAT_LIFETIME_MS),
        extras: None,
    };
    let bytes = beat
        .encode(own.scope(), own.pseudonym)
        .map_err(|e| format!("presence beat: {e}"))?;
    write_subkey(port, own, member_record::BEAT, bytes).await
}

/// Publish our member record in §4.4's order: the card (`M`/0, so the
/// record is openable), then `publish_ops`, which appends our ops to `M`/2
/// and writes `M`/3, then the beat (`M`/1).
///
/// # Errors
/// A step failed; nothing after it is written.
pub async fn publish_own_record<F>(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    profile: &OwnProfile,
    hpke_secret: &[u8; 32],
    publish_ops: F,
) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    write_card(port, own, profile, hpke_secret).await?;
    publish_ops.await?;
    write_beat(port, own, profile).await
}

/// A member's record as their `MemberJoined` names it.
pub struct MemberRecordRef<'a> {
    pub community_id: &'a str,
    pub pseudonym: &'a PseudonymKey,
    pub record: &'a str,
    /// `memberRecordOwner`.
    pub owner: &'a str,
}

/// Read `subkey` of `member.record`, refusing a record whose descriptor
/// owner is not `member.owner`.
async fn read_pinned(
    port: &dyn RecordPort,
    member: &MemberRecordRef<'_>,
    subkey: u32,
) -> Result<Option<Vec<u8>>, String> {
    let lease = port
        .acquire(member.record, None)
        .await
        .map_err(|e| format!("open member record: {e}"))?;
    let owner = port.owner_of(lease);
    let read = match owner {
        Ok(owner) if owner == member.owner => port
            .get(lease, subkey, true)
            .await
            .map_err(|e| format!("read member record subkey {subkey}: {e}")),
        Ok(owner) => Err(format!(
            "member record {} is owned by {owner}, its MemberJoined names {}",
            member.record, member.owner
        )),
        Err(e) => Err(format!("member record owner: {e}")),
    };
    port.release(lease).await;
    Ok(read?.map(|value| value.data))
}

fn scope<'a>(member: &MemberRecordRef<'a>) -> RecordScope<'a> {
    RecordScope {
        community_id: member.community_id,
        member_record: member.record,
    }
}

/// A member's verified card, `None` while `M`/0 holds none.
///
/// # Errors
/// The record's owner is not the one `MemberJoined` names, the read
/// failed, or the card does not verify under the member's pseudonym.
pub async fn read_card(
    port: &dyn RecordPort,
    member: &MemberRecordRef<'_>,
) -> Result<Option<SignedMemberCard>, String> {
    let Some(bytes) = read_pinned(port, member, member_record::CARD).await? else {
        return Ok(None);
    };
    decode_member_card(&bytes, scope(member), &member.pseudonym.0)
        .map(Some)
        .map_err(|e| format!("member card: {e}"))
}

/// A member's verified beat, `None` while `M`/1 holds none.
///
/// # Errors
/// As [`read_card`].
pub async fn read_beat(
    port: &dyn RecordPort,
    member: &MemberRecordRef<'_>,
) -> Result<Option<PresenceBeat>, String> {
    let Some(bytes) = read_pinned(port, member, member_record::BEAT).await? else {
        return Ok(None);
    };
    decode_presence_beat(&bytes, scope(member), &member.pseudonym.0)
        .map(Some)
        .map_err(|e| format!("presence beat: {e}"))
}

/// Read and remember the verified card of every pending member whose card
/// we do not hold yet, for the admission queue's names
/// (`GovernanceRuntimeDeps::member_card_name`). A card that does not
/// verify is not remembered.
pub async fn refresh_pending_cards<D: crate::deps::GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) {
    let Some(state) = deps.governance_state(community_id) else {
        return;
    };
    let Ok(port) = deps.record_port() else {
        return;
    };
    let mut pending: Vec<_> = state.pending_members.iter().collect();
    pending.sort_by_key(|(_, p)| p.stamp);
    for (pseudonym, member) in pending {
        if deps.member_card_name(community_id, pseudonym).is_some() {
            continue;
        }
        let at = MemberRecordRef {
            community_id,
            pseudonym,
            record: &member.member_record,
            owner: &member.member_record_owner,
        };
        match read_card(&*port, &at).await {
            Ok(Some(signed)) => {
                if let Some(name) = signed.card.display_name {
                    deps.note_member_card_name(community_id, pseudonym, name);
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(community = community_id, %error, "member card refused");
            }
        }
    }
}

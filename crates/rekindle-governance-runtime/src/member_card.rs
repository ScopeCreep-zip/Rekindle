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

use rekindle_records::port::OpenIntent;
use std::future::Future;

use rekindle_codec::member_record::{
    beat::decode_presence_beat, beat_number_at, card::decode_member_card, BeatState, MemberCard,
    PresenceBeat, RecordScope, SignedMemberCard,
};
use rekindle_records::port::{RecordPort, SetOutcome};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionStatus;
use rekindle_utils::random::RandomSource;
use zeroize::Zeroizing;

/// The card reads a refresh keeps in flight: half of Veilid's
/// `max_concurrent_operations` (16, veilid-core 0.5.8
/// `veilid_config.rs:329-336`), which gates every outbound get of the node
/// (`storage_manager/gated_operation.rs:404`). The refresh runs during a
/// join, beside the beat and `M`/6 reads of its bootstrap, presence and the
/// outbox streams; at 16, its card reads alone would take every slot and
/// those reads would queue behind them (plan E3.2 S7c.8 audit (8)).
pub const CARD_READS_IN_FLIGHT: usize = 8;

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
    /// Our beat's `seq`: the next of `communities.beat_seq`, persisted
    /// before the write (`GovernanceRuntimeDeps::next_beat_seq`).
    pub beat_seq: u64,
    /// Now, on the host's clock (ms).
    pub now_ms: u64,
    /// Our current general route blob, for `M`/6; empty while no route is
    /// allocated (the route publisher writes it once one is,
    /// [`crate::member_routes`]).
    pub route: Vec<u8>,
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

pub(crate) async fn write_subkey(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    subkey: u32,
    bytes: Vec<u8>,
) -> Result<(), String> {
    write_subkey_seq(port, own, subkey, bytes).await.map(drop)
}

/// [`write_subkey`], answering the subkey's DHT seq the write left in our
/// local copy (what a `HeadAdvanced` hint names).
async fn write_subkey_seq(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    subkey: u32,
    bytes: Vec<u8>,
) -> Result<u64, String> {
    let lease = port
        .acquire(own.record, Some(own.writer), OpenIntent::Hold)
        .await
        .map_err(|e| format!("open member record: {e}"))?;
    let written = port.set(lease, subkey, bytes, Some(own.writer)).await;
    let stored = match written.map_err(|e| format!("write member record subkey {subkey}: {e}")) {
        // An identical rewrite (`Unchanged`) is ours on the network too.
        Ok(SetOutcome::Landed | SetOutcome::Unchanged) => port
            .get(lease, subkey, false)
            .await
            .map_err(|e| format!("read back member record subkey {subkey}: {e}"))
            .and_then(|v| {
                v.map(|v| u64::from(v.seq))
                    .ok_or_else(|| format!("member record subkey {subkey} not in the local copy"))
            }),
        Ok(other) => Err(format!(
            "member record subkey {subkey} not stored: {other:?}"
        )),
        Err(e) => Err(e),
    };
    port.release(lease).await;
    stored
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

/// Write our first beat (`M`/1), without extras: `profile.beat_seq` and
/// the beat number of `profile.now_ms`.
///
/// # Errors
/// The beat does not encode, or the write did not land.
pub async fn write_beat(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    profile: &OwnProfile,
) -> Result<(), String> {
    let beat = OwnBeat {
        seq: profile.beat_seq,
        state: profile.status.into(),
        now_ms: profile.now_ms,
    };
    write_own_beat(port, own, &beat).await.map(drop)
}

/// One beat of ours, as the beat engine writes it (plan E3.4 S2).
pub struct OwnBeat {
    /// The next of `communities.beat_seq`, persisted before the write.
    pub seq: u64,
    pub state: BeatState,
    /// Now, on the host's clock (ms): the beat number's time.
    pub now_ms: u64,
}

/// Write `beat` to our `M`/1, without extras (padded by its encoder); the
/// subkey's DHT seq it left.
///
/// # Errors
/// The beat does not encode, or the write did not land.
pub async fn write_own_beat(
    port: &dyn RecordPort,
    own: &OwnRecord<'_>,
    beat: &OwnBeat,
) -> Result<u64, String> {
    let beat = PresenceBeat {
        seq: beat.seq,
        state: beat.state,
        voice_channel: None,
        beat_number: beat_number_at(beat.now_ms),
        extras: None,
    };
    let bytes = beat
        .encode(own.scope(), own.pseudonym)
        .map_err(|e| format!("presence beat: {e}"))?;
    write_subkey_seq(port, own, member_record::BEAT, bytes).await
}

/// Publish our member record in §4.4's order: the card (`M`/0, so the
/// record is openable) and our routes (`M`/6, so the record is reachable
/// once an op names it), then `publish_ops`, which appends our ops to `M`/2
/// and writes `M`/3, then the beat (`M`/1). An Invisible member writes no
/// beat at all (plan E3.4 "Constant rate"): its record shows nothing of its
/// presence until it picks a visible status.
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
    if !profile.route.is_empty() {
        crate::member_routes::write_routes(port, own, &profile.route, profile.now_ms).await?;
    }
    publish_ops.await?;
    if profile.status == SessionStatus::Invisible {
        return Ok(());
    }
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
    Ok(read_pinned_value(port, member, subkey)
        .await?
        .map(|value| value.data))
}

/// [`read_pinned`] with the value's DHT seq.
pub(crate) async fn read_pinned_value(
    port: &dyn RecordPort,
    member: &MemberRecordRef<'_>,
    subkey: u32,
) -> Result<Option<rekindle_records::port::RecordValue>, String> {
    let lease = port
        .acquire(member.record, None, OpenIntent::Check)
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
    read
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

/// Read and remember the verified card of every pending or admitted member
/// whose card name we do not hold yet: the admission queue's names and the
/// name a `Joined` lacked (plan E3.2 F9-2; the deps announce a name they
/// did not hold, `GovernanceRuntimeDeps::note_member_card_name`). Pending
/// members first, oldest first, then members by join stamp. A card that
/// does not verify is not remembered.
///
/// The reads run [`read_each`]'s way, [`CARD_READS_IN_FLIGHT`] in flight
/// (plan E3.2 S7c.9): a joiner holds no card name yet, so its first
/// rebuild reads a card per member, and one at a time that cost a forced
/// read per member on every join (≈ 0.5 s of simulated time each, the
/// join-cost run's growth per post-cut op).
pub async fn refresh_member_cards<D: crate::deps::GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) {
    let Some(state) = deps.governance_state(community_id) else {
        return;
    };
    let Ok(port) = deps.record_port() else {
        return;
    };
    // Our own card says our identity's name (`write_card`): a joiner's
    // first rebuild runs before it publishes the card, and a forced read of
    // the unwritten subkey is a fanout to its 10 s timeout (S7c.9).
    let own = deps.my_pseudonym(community_id);
    if let Some(own) = &own {
        if deps.member_card_name(community_id, own).is_none() {
            deps.note_member_card_name(community_id, own, deps.identity_display_name());
        }
    }
    let unnamed = cards_to_read(&state, community_id, own.as_ref(), |p| {
        deps.member_card_name(community_id, p).is_some()
    });
    let port = &*port;
    let mut reads = Vec::with_capacity(unnamed.len());
    for at in unnamed {
        reads.push((at.pseudonym, async move { read_card(port, &at).await }));
    }
    let read = read_each(reads).await;
    for (pseudonym, card) in read {
        match card {
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

/// The members whose card a refresh reads, pending ones first, each group
/// in stamp order: never departed members, us (`own`), or those whose name
/// we hold (`named`).
fn cards_to_read<'a>(
    state: &'a rekindle_governance::state::GovernanceState,
    community_id: &'a str,
    own: Option<&PseudonymKey>,
    named: impl Fn(&PseudonymKey) -> bool,
) -> Vec<MemberRecordRef<'a>> {
    let mut pending: Vec<_> = (state.pending_members.iter())
        .map(|(p, m)| (p, &m.member_record, &m.member_record_owner, m.stamp))
        .collect();
    pending.sort_by_key(|(_, _, _, stamp)| *stamp);
    let mut members: Vec<_> = (state.members.iter())
        .filter(|(p, _)| !state.is_departed(p))
        .map(|(p, m)| (p, &m.member_record, &m.member_record_owner, m.joined))
        .collect();
    members.sort_by_key(|(_, _, _, stamp)| *stamp);
    (pending.into_iter().chain(members))
        .filter(|(pseudonym, ..)| own != Some(*pseudonym) && !named(pseudonym))
        .map(|(pseudonym, record, owner, _)| MemberRecordRef {
            community_id,
            pseudonym,
            record,
            owner,
        })
        .collect()
}

/// Each of `reads` (a member's pseudonym and its card read), at most
/// [`CARD_READS_IN_FLIGHT`] in flight, answered in `reads`' order. The futures are built by the
/// caller (no closure in the returned future's type, which the Host's
/// `Send` bound cannot see through for every lifetime).
async fn read_each<T, Fut>(reads: Vec<(&PseudonymKey, Fut)>) -> Vec<(&PseudonymKey, T)>
where
    Fut: Future<Output = T>,
{
    use futures::stream::{self, StreamExt};
    let mut tagged = Vec::with_capacity(reads.len());
    for (pseudonym, read) in reads {
        tagged.push(async move { (pseudonym, read.await) });
    }
    stream::iter(tagged)
        .buffered(CARD_READS_IN_FLIGHT)
        .collect()
        .await
}

#[cfg(test)]
#[path = "member_card_tests.rs"]
mod tests;

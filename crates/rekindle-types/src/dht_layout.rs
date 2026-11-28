//! DHT record subkey layouts — the single source of truth for both
//! network tracks.
//!
//! The desktop track (`rekindle-protocol`) and the daemon track
//! (`rekindle-transport`) read and write the *same* DHT records, so a
//! subkey index means nothing unless both agree on it. They previously
//! kept private copies of this table and drifted: protocol allocated 9
//! subkeys with index 8 as the Strand Relay pool, while transport
//! allocated 10 with index 8 as the friend-inbox key. A profile written
//! by one and read by the other returned the wrong field entirely.
//!
//! Tier 1 (this crate) is the shared home precisely because both tracks
//! already depend on it and it has no dependencies of its own. Anything
//! that indexes a shared record belongs here — never in a track.

/// Largest `app_message` / `app_call` payload Veilid accepts:
/// `MAX_APP_MESSAGE_MESSAGE_LEN` and `MAX_APP_CALL_Q_MESSAGE_LEN`, both
/// 32,768 bytes (veilid-core 0.5.8 `rpc_processor/coders/operations/
/// operation_app_message.rs:3`, `operation_app_call.rs:3`, from
/// `proto/veilid.capnp:32-33`). Every encoded envelope a peer sends must
/// fit, so the sender refuses a larger one before Veilid sees it (plan
/// C7.25).
pub const MAX_APP_MESSAGE_LEN: usize = 32_768;

/// Largest value any DHT subkey holds: Veilid's `MAX_SUBKEY_SIZE`, 32,768
/// bytes (veilid-core 0.5.8 `veilid_api/types/dht/mod.rs:36`). A record's
/// per-subkey cap is this or its share of the record's data cap, whichever
/// is smaller (see [`prekeys`]).
pub const MAX_SUBKEY_SIZE: usize = 32_768;

/// Maximum member slots per registry segment.
///
/// Veilid's `DHTSchemaSMPL` has `MAX_MEMBER_COUNT = 256` and
/// `MAX_WRITER_COUNT = 256`. Under the v2.0 `o_cnt: 0` schema the
/// creation keypair owns no subkeys and is NOT counted as a writer
/// (`DHTSchemaSMPL::validate()` only does `writer_count += 1` when
/// `o_cnt > 0`), so 256 member slots would validate.
///
/// We allocate 255 anyway: the value sets Plate Gate segment
/// boundaries, so changing it reshards existing communities. One extra
/// slot is not worth a wire-visible migration.
pub const SLOTS_PER_SEGMENT: u32 = 255;

/// Subkey layout for a user's profile DHT record.
///
/// Indices 0-7 were always agreed. Index 8 is the Strand Relay pool
/// (its original protocol-side meaning, kept so existing desktop-written
/// records keep their semantics); the friend-inbox pair moved to 9/10,
/// which no reader had claimed for anything else.
#[path = "dht_layout_profile.rs"]
pub mod profile;

/// Subkey layout for the community governance manifest record.
///
/// Both tracks carried identical copies of this table. They happened to
/// agree, but nothing enforced it — the profile and registry tables in
/// the same two files did not.
#[path = "dht_layout_manifest.rs"]
pub mod manifest;

// The member registry record (SMPL, `o_cnt: 0`) has **no** layout
// module, and that absence is the v2.0 design rather than an omission.
//
// Every subkey is a member slot addressed by its raw index. The three
// community-wide entries that used to live at 0, 1 and 5 —
// `MEMBER_INDEX`, `MEK_VAULT`, `MODERATION_QUEUE` — were v1.0 owner
// subkeys, and `o_cnt: 0` credentials nobody to write them. Each has
// a v2.0 home: membership is derived by scanning signed presence rows
// (`rekindle_governance_runtime::roster`), the key is distributed
// peer-to-peer (`rekindle-mek-rotation`), and admission is a
// `GovernanceEntry`.

/// Subkey layout for a mailbox record.
#[path = "dht_layout_mailbox.rs"]
pub mod mailbox;

/// Subkey layout for a channel record (SMPL, `o_cnt: 0`).
#[path = "dht_layout_channel.rs"]
pub mod channel;

/// Subkey layout for a device's prekey record (plan E2.4): a DFLT(101)
/// record with an owner keypair of its own, named from profile subkey
/// [`profile::PREKEY_RECORDS`]. Subkey 0 is the `PrekeyRecordHead` (the
/// signed prekey and the ML-KEM last-resort key); subkeys 1..=100 each
/// hold one `OpkSlot` (a one-time curve key and a one-time ML-KEM key).
#[path = "dht_layout_prekeys.rs"]
pub mod prekeys;

/// Subkey layout for a DM participant record (plan E2.5): one DFLT(1)
/// record per participant per conversation, owned by a random keypair only
/// that participant holds. It anchors the participant's append-only chain
/// of sealed DFLT(1) pages (`rekindle_records::chain`). The retire mark
/// rides in the head, so a reader's diff is one fanout (A/B finding R5-5).
#[path = "dht_layout_dm_participant.rs"]
pub mod dm_participant;

/// Subkey layout for a friend invite's request inbox (plan E2.5, D17):
/// `SMPL(o_cnt: 1, [oneTimeWriter])`, a single-use capability. The invite
/// carries the one-time writer's keypair, so exactly one requester can
/// write [`request_inbox::REQUEST`]; Veilid refuses any other writer
/// (veilid-core 0.5.8 `veilid_api/types/dht/derivation.rs:208,233`
/// `check_subkey_value_data_smpl`, "wrong writer"). The owner subkey is the inviter's: it
/// writes [`request_inbox::OPENED`] there once at creation, which puts the
/// record on the network, so the requester's write finds its descriptor.
#[path = "dht_layout_request_inbox.rs"]
pub mod request_inbox;

/// Subkey layout for a group DM's rendezvous record (plan decision 67,
/// item 3): `SMPL(o_cnt: 1, members = 8 one-time writers, m_cnt: 8)`, the
/// durable path of a group member's first contact to a member who is not
/// their friend. The creator mints it with eight fresh writers; member
/// `i` of the roster holds writer `W_i` only (sealed to them in their
/// invite), so only they write row `i`: subkey [`subkey`]`(i, j)` is
/// member `i`'s first contact (a `PreKeyInit` frame) to member `j`, and
/// member `j` watches its column [`column`]`(j)`.
///
/// Veilid enforces the rows (veilid-core 0.5.8
/// `veilid_api/types/dht/derivation.rs:191-235`
/// `check_subkey_value_data_smpl`: the owner writes only subkey 0, and a
/// value in member `i`'s range must be signed by the writer whose member
/// id is `m_key`, else "wrong writer"). The creator, who minted every
/// writer, can write any row: it can only junk-write (deny the durable
/// path), never forge, since every entry is a signed `PreKeyInit` whose
/// sender the reader checks against the roster.
#[path = "dht_layout_group_rendezvous.rs"]
pub mod group_rendezvous;

/// Subkey layout for a community's anchor `A` (plan E3.2, ADR 0011): a
/// DFLT(1) record whose one subkey holds the `GenesisCert`, written once by
/// the anchor's owner `K_A`, whose keypair is then discarded. The community
/// id is `A`'s record key.
#[path = "dht_layout_community_anchor.rs"]
pub mod community_anchor;

/// Subkey layout for a member's `MemberRecord` `M(p)` (plan E3.2, D15,
/// ADR 0011): one DFLT(8) record per member per community, owned by a
/// random keypair `K_M(p)` only that member holds.
#[path = "dht_layout_member_record.rs"]
pub mod member_record;

/// Subkey layout for a community's sealed page (plan E3.2): an op-log,
/// spill or directory page, a DFLT(1) record written once through E2.5's
/// chain primitive (`rekindle_records::chain`), its owner keypair then
/// discarded.
#[path = "dht_layout_sealed_page.rs"]
pub mod sealed_page;

#[cfg(test)]
#[path = "dht_layout_tests.rs"]
mod tests;

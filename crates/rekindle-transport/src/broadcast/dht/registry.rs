//! Member registry SMPL record operations.
//!
//! v2.0 layout (`DHTSchema::smpl(0, members)`): the record has NO owner
//! subkeys — every subkey is a member slot addressed by its raw slot
//! index, and all 255 slot keypairs derive from one shared seed. This
//! matches how the desktop track builds the same records
//! (`rekindle_protocol::dht::schema::community_smpl_schema`) and how
//! `rekindle-presence` writes into them.
//!
//! This module used to hold the v1.0 layout: an 11-subkey owner block,
//! a `member_subkey()` that offset every slot past it, a DFLT(256)
//! creator-owned `create()`, and presence accessors built on the
//! offset. Records made that way are unreadable by everything else in
//! the system — slot N landed on subkey N+11 — so the offset, the
//! DFLT path and the unreachable presence accessors are gone, and
//! creation now goes through [`RegistryOps::create_segment`].
//!
//! **Known remaining v1.0 assumption.** The member index (subkey 0),
//! MEK vault (subkey 1) and moderation queue (subkey 5) accessors below
//! still address fixed low subkeys. Under `o_cnt: 0` those belong to
//! members 0, 1 and 5, so writes need that member's key and will not
//! succeed. Their v2.0 homes already exist — the member index is
//! derivable from the governance CRDT (`rekindle-governance`), MEK
//! distribution is peer-to-peer (`rekindle-mek-rotation`), and
//! moderation is a `GovernanceEntry` — so porting the ~50 call sites in
//! `rekindle-node` and `rekindle-transport::operations` onto them is
//! the remaining step.

use veilid_core::{DHTSchemaSMPLMember, KeyPair, RoutingContext};

use super::record;
use crate::error::{Result, TransportError};
use crate::payload::dht_types::SLOTS_PER_SEGMENT;

/// Operations on a community member registry.
pub struct RegistryOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> RegistryOps<'a> {
    pub fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a member registry under the v2.0 universal schema:
    /// `DHTSchema::smpl(0, members)` with one subkey per slot, all 255
    /// slot keypairs derived from `seed`.
    ///
    /// `o_cnt` is 0 deliberately. With any owner subkeys the creation
    /// keypair also counts as a writer
    /// (`DHTSchemaSMPL::validate` does `writer_count += 1` when
    /// `o_cnt > 0`), which is exactly the privileged writer flat
    /// governance exists to remove. Slot N is subkey N — no offset.
    pub async fn create_segment(&self, seed: &[u8; 32]) -> Result<(String, Option<KeyPair>)> {
        let mut members = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
        for slot in 0..SLOTS_PER_SEGMENT {
            let signing_key = derive_slot_keypair(seed, slot)?;
            let pub_bytes = signing_key.verifying_key().to_bytes();
            members.push(DHTSchemaSMPLMember {
                m_key: veilid_core::BareMemberId::new(&pub_bytes),
                m_cnt: 1,
            });
        }

        let (key, keypair) = record::create_smpl(self.rc, 0, members).await?;
        tracing::info!(key = %key, slots = SLOTS_PER_SEGMENT, "member registry created (o_cnt:0)");
        Ok((key, keypair))
    }

    // ── Open / Close ─────────────────────────────────────────────

    pub async fn open_writable(&self, key: &str, writer: KeyPair) -> Result<()> {
        record::open_writable(self.rc, key, writer).await
    }

    pub async fn open_readonly(&self, key: &str) -> Result<()> {
        record::open_readonly(self.rc, key).await
    }

    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }
}

// ── Slot keypair derivation ──────────────────────────────────────────

/// Derive a deterministic Ed25519 keypair for a SMPL member slot.
///
/// Delegates to `rekindle_secrets::derive::derive_slot_keypair` — the
/// single implementation shared by every track
/// (HKDF-SHA256(seed, "rekindle-slot-{index}") -> 32 bytes -> Ed25519).
pub fn derive_slot_keypair(seed: &[u8; 32], slot: u32) -> Result<ed25519_dalek::SigningKey> {
    rekindle_secrets::derive::derive_slot_keypair(seed, slot)
        .map_err(|e| TransportError::Internal(e.to_string()))
}

/// Derive the Veilid KeyPair for a SMPL member slot.
pub fn derive_slot_veilid_keypair(seed: &[u8; 32], slot: u32) -> Result<KeyPair> {
    let signing_key = derive_slot_keypair(seed, slot)?;
    let pub_bytes = signing_key.verifying_key().to_bytes();
    let secret_bytes = signing_key.to_bytes();

    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let veilid_pubkey = veilid_core::PublicKey::new(veilid_core::CRYPTO_KIND_VLD0, bare_pub);

    Ok(KeyPair::new_from_parts(veilid_pubkey, bare_secret))
}

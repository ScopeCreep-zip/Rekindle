//! A successor's sealed invites (plan E3.2 SS3c, lead decision (34)).
//!
//! When a creator publishes a successor it mints one single-use invite to
//! the new community per continuing member and writes them, each
//! `HPKE Base` sealed to that member's card `hpkeKey`
//! (`rekindle_secrets::hpke`, RFC 9180 X25519 / HKDF-SHA256 /
//! ChaCha20-Poly1305) under [`domains::SUCCESSOR_INVITE`], into sealed
//! DFLT(1) pages whose owner key is discarded after the one write (ADR 0011
//! "Sealed pages"). The op names only the first page (`invitesRecord`), so
//! no invite is readable in the op log, which removed members keep reading
//! (threat model). The AAD binds `old anchor ‖ 0x00 ‖ new anchor ‖ member`,
//! so an entry cannot be moved to another member, successor or community.
//!
//! Paging: Veilid caps a subkey at `MAX_SUBKEY_SIZE` = 32,768 bytes
//! (`veilid-core/src/veilid_api/types/dht/mod.rs:36`). An entry is at most
//! 32 + 32 + [`MAX_INVITE_LEN`] + 16 bytes of data plus its capnp struct
//! and pointers (about 900 bytes), and packing can grow incompressible
//! bytes by one tag byte in eight, so a page holds [`ENTRIES_PER_PAGE`] = 24
//! entries (under 25 KiB packed) and the first page also lists up to
//! [`MAX_PAGES`] - 1 further page keys; the pages hold up to 1,536 members.

use rekindle_types::domains;
use rekindle_types::id::PseudonymKey;

use crate::capnp_codec::{bounds, capnp_err, pack, text_to_string, unpack_bounded};
use crate::checkpoint_state::bytes32;
use crate::community_governance_capnp::successor_invites_page;
use crate::CodecError;

/// Longest invite link text an entry seals.
pub const MAX_INVITE_LEN: usize = 768;
/// Entries one page holds.
pub const ENTRIES_PER_PAGE: usize = 24;
/// Pages one successor's invites span, the first included.
pub const MAX_PAGES: usize = 64;

/// One continuing member's sealed invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedInvite {
    /// The member's pseudonym in the old community.
    pub member: PseudonymKey,
    pub enc: [u8; 32],
    pub ciphertext: Vec<u8>,
}

/// One page: the first lists the other pages' keys.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvitesPage {
    pub pages: Vec<String>,
    pub entries: Vec<SealedInvite>,
}

/// Why a successor invite could not be sealed or opened.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteSealError {
    #[error("the invite link is {0} bytes, over {MAX_INVITE_LEN}")]
    TooLong(usize),
    #[error("{0} continuing members need more than {MAX_PAGES} pages")]
    TooManyMembers(usize),
    #[error("HPKE: {0}")]
    Hpke(String),
    #[error("the sealed invite does not open with this member's card key")]
    NotOpened,
    #[error("the opened invite is not UTF-8 text")]
    NotText,
}

fn aad(old_anchor: &str, new_anchor: &str, member: &PseudonymKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(old_anchor.len() + new_anchor.len() + 33);
    out.extend_from_slice(old_anchor.as_bytes());
    out.push(0);
    out.extend_from_slice(new_anchor.as_bytes());
    out.extend_from_slice(&member.0);
    out
}

/// Seal `invite` to `member`'s card `hpke_key`.
///
/// # Errors
/// The invite is over [`MAX_INVITE_LEN`], or HPKE refused the key.
pub fn seal_invite(
    old_anchor: &str,
    new_anchor: &str,
    member: &PseudonymKey,
    hpke_key: &[u8; 32],
    invite: &str,
) -> Result<SealedInvite, InviteSealError> {
    if invite.len() > MAX_INVITE_LEN {
        return Err(InviteSealError::TooLong(invite.len()));
    }
    let sealed = rekindle_secrets::hpke::seal_base(
        hpke_key,
        domains::SUCCESSOR_INVITE,
        &aad(old_anchor, new_anchor, member),
        invite.as_bytes(),
    )
    .map_err(|e| InviteSealError::Hpke(e.to_string()))?;
    Ok(SealedInvite {
        member: member.clone(),
        enc: sealed.enc,
        ciphertext: sealed.ciphertext,
    })
}

/// Open `entry` with the member's card HPKE secret: the invite link text.
///
/// # Errors
/// The entry is not sealed to this secret for this member, successor and
/// community, or its plaintext is not text.
pub fn open_invite(
    old_anchor: &str,
    new_anchor: &str,
    entry: &SealedInvite,
    hpke_secret: &[u8; 32],
) -> Result<String, InviteSealError> {
    let plain = rekindle_secrets::hpke::open_base(
        hpke_secret,
        domains::SUCCESSOR_INVITE,
        &aad(old_anchor, new_anchor, &entry.member),
        &entry.enc,
        &entry.ciphertext,
    )
    .map_err(|_| InviteSealError::NotOpened)?;
    String::from_utf8(plain.to_vec()).map_err(|_| InviteSealError::NotText)
}

/// Split `entries` into pages of [`ENTRIES_PER_PAGE`]; the first page's
/// `pages` is filled by the writer once the others have keys.
///
/// # Errors
/// More than [`MAX_PAGES`] pages would be needed.
pub fn paginate(entries: Vec<SealedInvite>) -> Result<Vec<InvitesPage>, InviteSealError> {
    let count = entries.len();
    let mut pages: Vec<InvitesPage> = entries
        .chunks(ENTRIES_PER_PAGE)
        .map(|chunk| InvitesPage {
            pages: Vec::new(),
            entries: chunk.to_vec(),
        })
        .collect();
    if pages.len() > MAX_PAGES {
        return Err(InviteSealError::TooManyMembers(count));
    }
    if pages.is_empty() {
        pages.push(InvitesPage::default());
    }
    Ok(pages)
}

/// Encode one page.
#[must_use]
pub fn encode_invites_page(page: &InvitesPage) -> Vec<u8> {
    let mut message = capnp::message::Builder::new_default();
    let mut b = message.init_root::<successor_invites_page::Builder<'_>>();
    let n = u32::try_from(page.pages.len()).unwrap_or(u32::MAX);
    let mut keys = b.reborrow().init_pages(n);
    for (i, key) in (0..n).zip(&page.pages) {
        keys.set(i, key.as_str());
    }
    let n = u32::try_from(page.entries.len()).unwrap_or(u32::MAX);
    let mut entries = b.init_entries(n);
    for (i, entry) in (0..n).zip(&page.entries) {
        let mut e = entries.reborrow().get(i);
        e.set_member(&entry.member.0);
        e.set_enc(&entry.enc);
        e.set_ciphertext(&entry.ciphertext);
    }
    pack(&message)
}

/// Decode one page.
///
/// # Errors
/// Over the page bound, not a page, or a key or field malformed.
pub fn decode_invites_page(bytes: &[u8]) -> Result<InvitesPage, CodecError> {
    let message = unpack_bounded(bytes, bounds::SUCCESSOR_INVITES_BOUND)?;
    let r = message
        .get_root::<successor_invites_page::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let pages = (r.get_pages().map_err(|e| capnp_err(&e))?.iter())
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect::<Result<Vec<_>, _>>()?;
    let entries = (r.get_entries().map_err(|e| capnp_err(&e))?.iter())
        .map(|e| {
            Ok(SealedInvite {
                member: PseudonymKey(bytes32(e.get_member(), "invite member")?),
                enc: bytes32(e.get_enc(), "invite enc")?,
                ciphertext: e.get_ciphertext().map_err(|e| capnp_err(&e))?.to_vec(),
            })
        })
        .collect::<Result<Vec<_>, CodecError>>()?;
    Ok(InvitesPage { pages, entries })
}

#[cfg(test)]
#[path = "successor_invites_tests.rs"]
mod tests;

//! `M`/0, the member card: the per-community profile and the member's
//! HPKE key, cross-signed by its pseudonym (`rekindle-card-hpke-v1`) so the
//! key can be presented apart from the card, and the card signature
//! (`rekindle-member-card-v1`) over the exact body bytes.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::domains;
use rekindle_types::presence::GameInfo;

use super::{check_len, check_size, opt_text, put_lp, sign, verify, RecordScope};
use crate::capnp_codec::{capnp_err, pack, text_to_string, unpack_bounded};
use crate::member_record_capnp::{card_game, member_card, member_card_body};
use crate::CodecError;

/// The bound on an encoded card, proven at the maximum field sizes.
pub const MEMBER_CARD_MAX_BYTES: usize = 8 * 1024;
/// Field bounds, in UTF-8 bytes.
pub const MAX_DISPLAY_NAME: usize = 256;
pub const MAX_BIO: usize = 2048;
pub const MAX_REF: usize = 512;
pub const MAX_PRONOUNS: usize = 160;
pub const MAX_CUSTOM_STATUS: usize = 512;
pub const MAX_BADGES: usize = 16;
pub const MAX_BADGE: usize = 64;
pub const MAX_GAME_NAME: usize = 256;
pub const MAX_GAME_ID: usize = 64;
pub const MAX_SERVER_ADDRESS: usize = 256;

const MAX_WORDS: usize = 4 * 1024;

/// A member's per-community profile.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberCard {
    pub display_name: Option<String>,
    pub bio: Option<String>,
    pub avatar_ref: Option<String>,
    pub banner_ref: Option<String>,
    pub pronouns: Option<String>,
    /// ARGB.
    pub theme_color: Option<u32>,
    pub badges: Vec<String>,
    pub custom_status: Option<String>,
    pub game: Option<GameInfo>,
    /// The member's X25519 HPKE public key in this community.
    pub hpke_key: [u8; 32],
}

/// A verified card and the pseudonym's cross-signature over its HPKE key.
#[derive(Debug, Clone, PartialEq)]
pub struct SignedMemberCard {
    pub card: MemberCard,
    pub hpke_sig: [u8; 64],
}

/// `rekindle-card-hpke-v1 ‖ community ‖ hpkeKey`.
#[must_use]
pub fn card_hpke_signing_bytes(community_id: &str, hpke_key: &[u8; 32]) -> Vec<u8> {
    let mut out = domains::CARD_HPKE.as_bytes().to_vec();
    put_lp(&mut out, community_id.as_bytes());
    out.extend_from_slice(hpke_key);
    out
}

/// Verify the pseudonym's cross-signature over `hpke_key` in `community_id`.
pub fn verify_card_hpke(
    community_id: &str,
    hpke_key: &[u8; 32],
    hpke_sig: &[u8; 64],
    pseudonym: &[u8; 32],
) -> Result<(), CodecError> {
    verify(
        pseudonym,
        &card_hpke_signing_bytes(community_id, hpke_key),
        hpke_sig,
        "card hpke key",
    )
}

fn opt_len(v: Option<&String>) -> usize {
    v.map_or(0, String::len)
}

impl MemberCard {
    fn check_bounds(&self) -> Result<(), CodecError> {
        check_len(
            "display name",
            opt_len(self.display_name.as_ref()),
            MAX_DISPLAY_NAME,
        )?;
        check_len("bio", opt_len(self.bio.as_ref()), MAX_BIO)?;
        check_len("avatar ref", opt_len(self.avatar_ref.as_ref()), MAX_REF)?;
        check_len("banner ref", opt_len(self.banner_ref.as_ref()), MAX_REF)?;
        check_len("pronouns", opt_len(self.pronouns.as_ref()), MAX_PRONOUNS)?;
        check_len(
            "custom status",
            opt_len(self.custom_status.as_ref()),
            MAX_CUSTOM_STATUS,
        )?;
        check_len("badge count", self.badges.len(), MAX_BADGES)?;
        for badge in &self.badges {
            check_len("badge", badge.len(), MAX_BADGE)?;
        }
        if let Some(game) = &self.game {
            check_len("game name", game.game_name.len(), MAX_GAME_NAME)?;
            check_len("game id", opt_len(game.game_id.as_ref()), MAX_GAME_ID)?;
            check_len(
                "server address",
                opt_len(game.server_address.as_ref()),
                MAX_SERVER_ADDRESS,
            )?;
        }
        Ok(())
    }

    fn encode_body(&self, hpke_sig: &[u8; 64]) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<member_card_body::Builder<'_>>();
            let texts = [
                self.display_name.as_deref(),
                self.bio.as_deref(),
                self.avatar_ref.as_deref(),
                self.banner_ref.as_deref(),
                self.pronouns.as_deref(),
                self.custom_status.as_deref(),
            ];
            let [name, bio, avatar, banner, pronouns, custom] = texts;
            if let Some(v) = name {
                b.set_display_name(v);
            }
            if let Some(v) = bio {
                b.set_bio(v);
            }
            if let Some(v) = avatar {
                b.set_avatar_ref(v);
            }
            if let Some(v) = banner {
                b.set_banner_ref(v);
            }
            if let Some(v) = pronouns {
                b.set_pronouns(v);
            }
            if let Some(v) = custom {
                b.set_custom_status(v);
            }
            if let Some(color) = self.theme_color {
                b.set_has_theme_color(true);
                b.set_theme_color(color);
            }
            let count = u32::try_from(self.badges.len()).unwrap_or(u32::MAX);
            let mut badges = b.reborrow().init_badges(count);
            for (i, badge) in (0u32..).zip(&self.badges) {
                badges.set(i, badge.as_str());
            }
            if let Some(game) = &self.game {
                write_game(b.reborrow().init_game(), game);
            }
            b.set_hpke_key(&self.hpke_key);
            b.set_hpke_sig(hpke_sig);
        }
        pack(&builder)
    }

    /// Sign and encode the card for the record `at`, refusing a field over
    /// its bound.
    pub fn encode(
        &self,
        at: RecordScope<'_>,
        pseudonym: &SigningKey,
    ) -> Result<Vec<u8>, CodecError> {
        self.check_bounds()?;
        let hpke_sig = sign(
            pseudonym,
            &card_hpke_signing_bytes(at.community_id, &self.hpke_key),
        );
        let body = self.encode_body(&hpke_sig);
        let mut signed = at.signing_prefix(domains::MEMBER_CARD);
        signed.extend_from_slice(&body);
        let sig = sign(pseudonym, &signed);
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<member_card::Builder<'_>>();
            root.set_body(&body);
            root.set_sig(&sig);
        }
        let out = pack(&builder);
        check_size("member card", &out, MEMBER_CARD_MAX_BYTES)?;
        Ok(out)
    }
}

fn write_game(mut g: card_game::Builder<'_>, game: &GameInfo) {
    g.set_game_name(game.game_name.as_str());
    if let Some(id) = &game.game_id {
        g.set_game_id(id.as_str());
    }
    if let Some(elapsed) = game.elapsed_seconds {
        g.set_has_elapsed_seconds(true);
        g.set_elapsed_seconds(elapsed);
    }
    if let Some(addr) = &game.server_address {
        g.set_server_address(addr.as_str());
    }
}

fn read_game(g: card_game::Reader<'_>) -> Result<GameInfo, CodecError> {
    Ok(GameInfo {
        game_name: text_to_string(g.get_game_name().map_err(|e| capnp_err(&e))?)?,
        game_id: opt_text(g.has_game_id(), g.get_game_id())?,
        elapsed_seconds: g.get_has_elapsed_seconds().then(|| g.get_elapsed_seconds()),
        server_address: opt_text(g.has_server_address(), g.get_server_address())?,
    })
}

/// Decode `M`/0 and verify both signatures by `pseudonym` for the record
/// `at`.
pub fn decode_member_card(
    data: &[u8],
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<SignedMemberCard, CodecError> {
    check_size("member card", data, MEMBER_CARD_MAX_BYTES)?;
    let outer = unpack_bounded(data, MAX_WORDS)?;
    let root = outer
        .get_root::<member_card::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let body = root.get_body().map_err(|e| capnp_err(&e))?;
    let mut signed = at.signing_prefix(domains::MEMBER_CARD);
    signed.extend_from_slice(body);
    verify(
        pseudonym,
        &signed,
        root.get_sig().map_err(|e| capnp_err(&e))?,
        "member card",
    )?;

    let inner = unpack_bounded(body, MAX_WORDS)?;
    let b = inner
        .get_root::<member_card_body::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let hpke_key: [u8; 32] = b
        .get_hpke_key()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization("card hpke key length".into()))?;
    let hpke_sig: [u8; 64] = b
        .get_hpke_sig()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization("card hpke sig length".into()))?;
    verify_card_hpke(at.community_id, &hpke_key, &hpke_sig, pseudonym)?;

    let badges = b
        .get_badges()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect::<Result<Vec<_>, _>>()?;
    let card = MemberCard {
        display_name: opt_text(b.has_display_name(), b.get_display_name())?,
        bio: opt_text(b.has_bio(), b.get_bio())?,
        avatar_ref: opt_text(b.has_avatar_ref(), b.get_avatar_ref())?,
        banner_ref: opt_text(b.has_banner_ref(), b.get_banner_ref())?,
        pronouns: opt_text(b.has_pronouns(), b.get_pronouns())?,
        theme_color: b.get_has_theme_color().then(|| b.get_theme_color()),
        badges,
        custom_status: opt_text(b.has_custom_status(), b.get_custom_status())?,
        game: if b.has_game() {
            Some(read_game(b.get_game().map_err(|e| capnp_err(&e))?)?)
        } else {
            None
        },
        hpke_key,
    };
    card.check_bounds()
        .map_err(|e| CodecError::Deserialization(e.to_string()))?;
    Ok(SignedMemberCard { card, hpke_sig })
}

//! `governance` expression variant encode/decode helpers.

use super::super::len_u32;
use super::super::sub_types::{
    pseudonym_key_from_capnp, pseudonym_key_to_capnp, uuid16_from_capnp, uuid16_to_capnp,
};
use super::shared::{read_sound_meta, write_sound_meta};
use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::{self as schema_pkg};
use crate::error::ProtocolError;
use rekindle_types::governance::GovernanceEntry;

pub(super) fn write_expression_added(
    mut p: schema_pkg::expression_added_entry::Builder<'_>,
    e: &GovernanceEntry,
) {
    let GovernanceEntry::ExpressionAdded {
        expression_id,
        name,
        kind,
        content_hash,
        attachment,
        animated,
        tags,
        sound_meta,
        creator_pseudonym,
        created_at,
        available_to_peers,
        lamport,
    } = e
    else {
        unreachable!("write_expression_added: variant mismatch")
    };
    uuid16_to_capnp(p.reborrow().init_expression_id(), expression_id);
    p.set_name(name);
    p.set_kind(kind);
    p.set_content_hash(content_hash);
    // Architecture §18.4 — encoders never write the deprecated
    // inline_data path. Bytes travel via the AttachmentOffer + Lost Cargo.
    p.set_has_inline_data(false);
    p.set_animated(*animated);
    let mut tag_list = p.reborrow().init_tags(len_u32(tags.len()));
    for (i, t) in tags.iter().enumerate() {
        tag_list.set(len_u32(i), t.as_str());
    }
    p.set_has_sound_meta(sound_meta.is_some());
    if let Some(sm) = sound_meta {
        write_sound_meta(p.reborrow().init_sound_meta(), sm);
    }
    p.set_has_creator_pseudonym(creator_pseudonym.is_some());
    if let Some(c) = creator_pseudonym {
        pseudonym_key_to_capnp(p.reborrow().init_creator_pseudonym(), c);
    }
    p.set_has_created_at(created_at.is_some());
    if let Some(t) = created_at {
        p.set_created_at(*t);
    }
    p.set_has_available_to_peers(available_to_peers.is_some());
    if let Some(a) = available_to_peers {
        p.set_available_to_peers(*a);
    }
    p.set_lamport(*lamport);
    p.set_has_attachment(attachment.is_some());
    if let Some(offer) = attachment {
        write_attachment_offer(p.reborrow().init_attachment(), offer);
    }
}

pub(super) fn write_attachment_offer(
    mut p: schema_pkg::attachment_offer::Builder<'_>,
    offer: &rekindle_types::attachment::AttachmentOffer,
) {
    uuid16_to_capnp(p.reborrow().init_attachment_id(), &offer.attachment_id);
    p.set_filename(&offer.filename);
    p.set_mime_type(&offer.mime_type);
    p.set_total_size(offer.total_size);
    p.set_chunk_count(offer.chunk_count);
    p.set_chunk_size(offer.chunk_size);
    p.set_merkle_root(&offer.merkle_root);
    let mut hashes = p
        .reborrow()
        .init_chunk_hashes(len_u32(offer.chunk_hashes.len()));
    for (i, h) in offer.chunk_hashes.iter().enumerate() {
        hashes.set(len_u32(i), h);
    }
    p.set_wrapped_fek(&offer.wrapped_fek);
    p.set_fek_mek_generation(offer.fek_mek_generation);
}

pub(super) fn write_expression_removed(
    mut p: schema_pkg::expression_removed_entry::Builder<'_>,
    expression_id: &[u8; 16],
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_expression_id(), expression_id);
    p.set_lamport(lamport);
}

pub(super) fn write_attachment_pinned(
    mut p: schema_pkg::attachment_pinned_entry::Builder<'_>,
    attachment_id: &[u8; 16],
    pinned: bool,
    lamport: u64,
) {
    uuid16_to_capnp(p.reborrow().init_attachment_id(), attachment_id);
    p.set_pinned(pinned);
    p.set_lamport(lamport);
}

pub(super) fn read_expression_added(
    p: schema_pkg::expression_added_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    let tags: Result<Vec<String>, ProtocolError> = p
        .get_tags()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect();
    // Architecture §18.4 — readers prefer the new `attachment` field.
    // The deprecated inline_data path is silently dropped; receivers
    // who can't reach the AttachmentOffer chunks will see a missing
    // asset (handled by the eager-fetch loop on next governance merge).
    let attachment = if p.get_has_attachment() {
        Some(read_attachment_offer(
            p.get_attachment().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    Ok(GovernanceEntry::ExpressionAdded {
        expression_id: uuid16_from_capnp(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        name: text_to_string(p.get_name().map_err(|e| capnp_err(&e))?)?,
        kind: text_to_string(p.get_kind().map_err(|e| capnp_err(&e))?)?,
        content_hash: text_to_string(p.get_content_hash().map_err(|e| capnp_err(&e))?)?,
        attachment,
        animated: p.get_animated(),
        tags: tags?,
        sound_meta: if p.get_has_sound_meta() {
            Some(read_sound_meta(
                p.get_sound_meta().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        creator_pseudonym: if p.get_has_creator_pseudonym() {
            Some(pseudonym_key_from_capnp(
                p.get_creator_pseudonym().map_err(|e| capnp_err(&e))?,
            )?)
        } else {
            None
        },
        created_at: if p.get_has_created_at() {
            Some(p.get_created_at())
        } else {
            None
        },
        available_to_peers: if p.get_has_available_to_peers() {
            Some(p.get_available_to_peers())
        } else {
            None
        },
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_attachment_offer(
    p: schema_pkg::attachment_offer::Reader<'_>,
) -> Result<rekindle_types::attachment::AttachmentOffer, ProtocolError> {
    let chunk_hashes: Result<Vec<[u8; 32]>, ProtocolError> = p
        .get_chunk_hashes()
        .map_err(|e| capnp_err(&e))?
        .iter()
        .map(|h| {
            let bytes = h.map_err(|e| capnp_err(&e))?;
            bytes.try_into().map_err(|_| {
                ProtocolError::Deserialization("attachment chunk hash not 32 bytes".into())
            })
        })
        .collect();
    let merkle_root: [u8; 32] = p
        .get_merkle_root()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| {
            ProtocolError::Deserialization("attachment merkle_root not 32 bytes".into())
        })?;
    Ok(rekindle_types::attachment::AttachmentOffer {
        attachment_id: uuid16_from_capnp(p.get_attachment_id().map_err(|e| capnp_err(&e))?)?,
        filename: text_to_string(p.get_filename().map_err(|e| capnp_err(&e))?)?,
        mime_type: text_to_string(p.get_mime_type().map_err(|e| capnp_err(&e))?)?,
        total_size: p.get_total_size(),
        chunk_count: p.get_chunk_count(),
        chunk_size: p.get_chunk_size(),
        merkle_root,
        chunk_hashes: chunk_hashes?,
        wrapped_fek: p.get_wrapped_fek().map_err(|e| capnp_err(&e))?.to_vec(),
        fek_mek_generation: p.get_fek_mek_generation(),
    })
}

pub(super) fn read_expression_removed(
    p: schema_pkg::expression_removed_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::ExpressionRemoved {
        expression_id: uuid16_from_capnp(p.get_expression_id().map_err(|e| capnp_err(&e))?)?,
        lamport: p.get_lamport(),
    })
}

pub(super) fn read_attachment_pinned(
    p: schema_pkg::attachment_pinned_entry::Reader<'_>,
) -> Result<GovernanceEntry, ProtocolError> {
    Ok(GovernanceEntry::AttachmentPinned {
        attachment_id: uuid16_from_capnp(p.get_attachment_id().map_err(|e| capnp_err(&e))?)?,
        pinned: p.get_pinned(),
        lamport: p.get_lamport(),
    })
}

//! The governance `CommunitySuccessor` op body (plan E3.2 S7s.3e).

use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{
    community_successor_entry, creator_link, successor_link_entry, successor_link_request_entry,
    SuccessorReason as Reason,
};
use crate::error::CodecError;
use crate::governance_op::{read_op_ref, write_op_ref};
use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, SuccessorLink, SuccessorLinkRequest,
    SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

pub(super) fn read_community_successor(
    p: community_successor_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    let anchor = text_to_string(p.get_new_anchor().map_err(|e| capnp_err(&e))?)?;
    let reason = match p.get_reason().map_err(not_in_schema)? {
        Reason::Recreate => SuccessorReason::Recreate,
        Reason::Transfer => SuccessorReason::Transfer,
    };
    let accepts = if p.has_accepts() {
        Some(read_op_ref(p.get_accepts().map_err(|e| capnp_err(&e))?)?)
    } else {
        None
    };
    let links = (p.get_links().map_err(|e| capnp_err(&e))?.iter())
        .map(read_creator_link)
        .collect::<Result<Vec<_>, CodecError>>()?;
    Ok(GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: (!anchor.is_empty()).then_some(anchor),
        reason,
        transfer_to: optional(p.get_transfer_to(), "transferTo")?.map(PseudonymKey),
        creator_set_hash: optional(p.get_creator_set_hash(), "creatorSetHash")?,
        accepts,
        links,
        invites_record: {
            let record = text_to_string(p.get_invites_record().map_err(|e| capnp_err(&e))?)?;
            (!record.is_empty()).then_some(record)
        },
    }))
}

pub(super) fn write_community_successor(
    mut p: community_successor_entry::Builder<'_>,
    s: &CommunitySuccessor,
) {
    p.set_new_anchor(s.new_anchor.as_deref().unwrap_or_default());
    p.set_invites_record(s.invites_record.as_deref().unwrap_or_default());
    p.set_reason(match s.reason {
        SuccessorReason::Recreate => Reason::Recreate,
        SuccessorReason::Transfer => Reason::Transfer,
    });
    if let Some(to) = &s.transfer_to {
        p.set_transfer_to(&to.0);
    }
    if let Some(hash) = &s.creator_set_hash {
        p.set_creator_set_hash(hash);
    }
    if let Some(accepts) = &s.accepts {
        write_op_ref(p.reborrow().init_accepts(), accepts);
    }
    let n = u32::try_from(s.links.len()).unwrap_or(u32::MAX);
    let mut links = p.init_links(n);
    for (i, link) in (0..n).zip(&s.links) {
        write_creator_link(links.reborrow().get(i), link);
    }
}

fn read_creator_link(l: creator_link::Reader<'_>) -> Result<CreatorLink, CodecError> {
    Ok(CreatorLink {
        old: PseudonymKey(fixed(l.get_old(), "link old")?),
        new: PseudonymKey(fixed(l.get_new(), "link new")?),
        sig: l.get_sig().map_err(|e| capnp_err(&e))?.to_vec(),
    })
}

fn write_creator_link(mut l: creator_link::Builder<'_>, link: &CreatorLink) {
    l.set_old(&link.old.0);
    l.set_new(&link.new.0);
    l.set_sig(&link.sig);
}

/// The link round's request (lead decision (46)).
pub(super) fn read_link_request(
    p: successor_link_request_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    let asked = (p.get_asked().map_err(|e| capnp_err(&e))?.iter())
        .map(|k| fixed(k, "asked").map(PseudonymKey))
        .collect::<Result<Vec<_>, CodecError>>()?;
    Ok(GovernanceEntry::SuccessorLinkRequest(
        SuccessorLinkRequest {
            offer: read_op_ref(p.get_offer().map_err(|e| capnp_err(&e))?)?,
            new_anchor: text_to_string(p.get_new_anchor().map_err(|e| capnp_err(&e))?)?,
            asked,
        },
    ))
}

pub(super) fn write_link_request(
    mut p: successor_link_request_entry::Builder<'_>,
    r: &SuccessorLinkRequest,
) {
    write_op_ref(p.reborrow().init_offer(), &r.offer);
    p.set_new_anchor(r.new_anchor.as_str());
    let n = u32::try_from(r.asked.len()).unwrap_or(u32::MAX);
    let mut asked = p.init_asked(n);
    for (i, k) in (0..n).zip(&r.asked) {
        asked.set(i, &k.0);
    }
}

/// A co-creator's link answering a request (lead decision (46)).
pub(super) fn read_link(
    p: successor_link_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    Ok(GovernanceEntry::SuccessorLink(SuccessorLink {
        request: read_op_ref(p.get_request().map_err(|e| capnp_err(&e))?)?,
        link: read_creator_link(p.get_link().map_err(|e| capnp_err(&e))?)?,
        creator_sig: p.get_creator_sig().map_err(|e| capnp_err(&e))?.to_vec(),
    }))
}

pub(super) fn write_link(mut p: successor_link_entry::Builder<'_>, l: &SuccessorLink) {
    write_op_ref(p.reborrow().init_request(), &l.request);
    write_creator_link(p.reborrow().init_link(), &l.link);
    p.set_creator_sig(&l.creator_sig);
}

/// A 32-byte field; any other length is refused.
fn fixed(get: Result<&[u8], capnp::Error>, what: &str) -> Result<[u8; 32], CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!("successor {what} is {} bytes, not 32", bytes.len()))
    })
}

/// An optional 32-byte field: empty is unset, any other length but 32 is
/// refused.
fn optional(get: Result<&[u8], capnp::Error>, what: &str) -> Result<Option<[u8; 32]>, CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    fixed(Ok(bytes), what).map(Some)
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;

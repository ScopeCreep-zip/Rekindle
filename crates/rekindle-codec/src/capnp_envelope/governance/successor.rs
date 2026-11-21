//! The governance `CommunitySuccessor` op body (plan E3.2 S7s.3e).

use crate::capnp_codec::{capnp_err, not_in_schema, text_to_string};
use crate::community_governance_capnp::{community_successor_entry, SuccessorReason as Reason};
use crate::error::CodecError;
use crate::governance_op::{read_op_ref, write_op_ref};
use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, SuccessorReason,
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
        .map(|l| {
            Ok(CreatorLink {
                old: PseudonymKey(fixed(l.get_old(), "link old")?),
                new: PseudonymKey(fixed(l.get_new(), "link new")?),
                sig: l.get_sig().map_err(|e| capnp_err(&e))?.to_vec(),
            })
        })
        .collect::<Result<Vec<_>, CodecError>>()?;
    Ok(GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: (!anchor.is_empty()).then_some(anchor),
        reason,
        transfer_to: optional(p.get_transfer_to(), "transferTo")?.map(PseudonymKey),
        creator_set_hash: optional(p.get_creator_set_hash(), "creatorSetHash")?,
        accepts,
        links,
    }))
}

pub(super) fn write_community_successor(
    mut p: community_successor_entry::Builder<'_>,
    s: &CommunitySuccessor,
) {
    p.set_new_anchor(s.new_anchor.as_deref().unwrap_or_default());
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
        let mut l = links.reborrow().get(i);
        l.set_old(&link.old.0);
        l.set_new(&link.new.0);
        l.set_sig(&link.sig);
    }
}

/// A 32-byte field; any other length is refused.
fn fixed(get: Result<&[u8], capnp::Error>, what: &str) -> Result<[u8; 32], CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!(
            "successor {what} is {} bytes, not 32",
            bytes.len()
        ))
    })
}

/// An optional 32-byte field: empty is unset, any other length but 32 is
/// refused.
fn optional(
    get: Result<&[u8], capnp::Error>,
    what: &str,
) -> Result<Option<[u8; 32]>, CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    fixed(Ok(bytes), what).map(Some)
}

#[cfg(test)]
mod tests {
    use rekindle_types::governance::{
        CommunitySuccessor, CreatorLink, GovernanceEntry, OpHash, OpRef, SuccessorReason,
    };
    use rekindle_types::id::PseudonymKey;

    use super::super::{decode_governance_entry, encode_governance_entry};

    fn body(anchor: bool, to: bool, set: bool, accepts: bool, links: u8) -> GovernanceEntry {
        GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
            new_anchor: anchor.then(|| format!("VLD0:{}", "a".repeat(43))),
            reason: if to || set {
                SuccessorReason::Transfer
            } else {
                SuccessorReason::Recreate
            },
            transfer_to: to.then_some(PseudonymKey([7; 32])),
            creator_set_hash: set.then_some([8; 32]),
            accepts: accepts.then(|| OpRef {
                op_hash: OpHash([9; 32]),
                author: PseudonymKey([10; 32]),
                author_record: format!("VLD0:{}", "r".repeat(43)),
                author_seq: 3,
            }),
            links: (0..links)
                .map(|i| CreatorLink {
                    old: PseudonymKey([i; 32]),
                    new: PseudonymKey([i + 100; 32]),
                    sig: vec![i; 64],
                })
                .collect(),
        })
    }

    #[test]
    fn community_successor_round_trips() {
        let all = [
            body(true, false, false, false, 0),
            body(false, true, true, false, 0),
            body(false, false, false, false, 0),
            body(true, false, true, true, 9),
        ];
        for entry in all {
            let bytes = encode_governance_entry(&entry);
            assert_eq!(decode_governance_entry(&bytes).expect("decodes"), entry);
        }
    }
}

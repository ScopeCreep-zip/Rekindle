//! The governance `Checkpoint` op body (plan E3.2 S7c.3).

use crate::capnp_codec::{capnp_err, text_to_string};
use crate::community_governance_capnp::checkpoint_entry;
use crate::error::CodecError;
use crate::governance_op::{read_op_ref, write_op_ref};
use rekindle_types::governance::{Checkpoint, GovernanceEntry};

pub(super) fn read_checkpoint(
    p: checkpoint_entry::Reader<'_>,
) -> Result<GovernanceEntry, CodecError> {
    let prev = if p.get_has_prev_checkpoint() {
        Some(read_op_ref(
            p.get_prev_checkpoint().map_err(|e| capnp_err(&e))?,
        )?)
    } else {
        None
    };
    let cut = (p.get_cut().map_err(|e| capnp_err(&e))?.iter())
        .map(read_op_ref)
        .collect::<Result<Vec<_>, _>>()?;
    let pages = (p.get_pages().map_err(|e| capnp_err(&e))?.iter())
        .map(|t| text_to_string(t.map_err(|e| capnp_err(&e))?))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GovernanceEntry::Checkpoint(Checkpoint {
        height: p.get_height(),
        round: p.get_round(),
        prev,
        cut,
        frontier_hash: hash32(p.get_frontier_hash(), "frontierHash")?,
        dir_root: hash32(p.get_dir_root(), "dirRoot")?,
        dir_size: p.get_dir_size(),
        state_root: hash32(p.get_state_root(), "stateRoot")?,
        state_size: p.get_state_size(),
        mek_generation: p.get_mek_generation(),
        eligible_set_hash: hash32(p.get_eligible_set_hash(), "eligibleSetHash")?,
        pages,
    }))
}

pub(super) fn write_checkpoint(mut p: checkpoint_entry::Builder<'_>, c: &Checkpoint) -> Result<(), CodecError> {
    p.set_height(c.height);
    p.set_round(c.round);
    if let Some(prev) = &c.prev {
        p.set_has_prev_checkpoint(true);
        write_op_ref(p.reborrow().init_prev_checkpoint(), prev);
    }
    let mut cut = p.reborrow().init_cut(crate::capnp_codec::wire_len(c.cut.len())?);
    for (i, head) in (0u32..).zip(c.cut.iter()) {
        write_op_ref(cut.reborrow().get(i), head);
    }
    p.set_frontier_hash(&c.frontier_hash);
    p.set_dir_root(&c.dir_root);
    p.set_dir_size(c.dir_size);
    p.set_state_root(&c.state_root);
    p.set_state_size(c.state_size);
    p.set_mek_generation(c.mek_generation);
    p.set_eligible_set_hash(&c.eligible_set_hash);
    let mut pages = p.init_pages(crate::capnp_codec::wire_len(c.pages.len())?);
    for (i, page) in (0u32..).zip(c.pages.iter()) {
        pages.set(i, page.as_str());
    }
    Ok(())
}

/// A 32-byte digest field; any other length is refused.
fn hash32(get: Result<&[u8], capnp::Error>, what: &str) -> Result<[u8; 32], CodecError> {
    let bytes = get.map_err(|e| capnp_err(&e))?;
    bytes.try_into().map_err(|_| {
        CodecError::Deserialization(format!(
            "checkpoint {what} is {} bytes, not 32",
            bytes.len()
        ))
    })
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;

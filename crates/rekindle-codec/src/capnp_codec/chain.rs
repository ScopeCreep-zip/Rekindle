//! The append-only chain's values (plan E2.5, primitive N2;
//! `schemas/chain.capnp`). This is the wire shape only: whether a page fits
//! its pointer and the page before it is `rekindle_records::chain`'s check.

use super::direct::data;
use super::{capnp_err, pack, text_to_string, unpack, CodecError};
use crate::chain_capnp;

/// Where a page sits: its record key and the sequence numbers it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePointer {
    pub page_key: String,
    pub first_seq: u64,
    pub last_seq: u64,
}

/// One sealed page: its entries from `first_seq` up, and the page before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainPage {
    /// `None` on the chain's first page.
    pub prev: Option<PagePointer>,
    pub first_seq: u64,
    /// Opaque ciphertexts, in sequence order.
    pub entries: Vec<Vec<u8>>,
}

fn set_pointer(mut b: chain_capnp::page_pointer::Builder<'_>, p: &PagePointer) {
    b.set_page_key(p.page_key.as_str());
    b.set_first_seq(p.first_seq);
    b.set_last_seq(p.last_seq);
}

fn read_pointer(r: chain_capnp::page_pointer::Reader<'_>) -> Result<PagePointer, CodecError> {
    Ok(PagePointer {
        page_key: text_to_string(r.get_page_key().map_err(|e| capnp_err(&e))?)?,
        first_seq: r.get_first_seq(),
        last_seq: r.get_last_seq(),
    })
}

/// Participant record subkey `HEAD`: the newest page.
#[must_use]
pub fn encode_chain_head(newest: &PagePointer) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let root = builder.init_root::<chain_capnp::chain_head::Builder<'_>>();
        set_pointer(root.init_newest(), newest);
    }
    pack(&builder)
}

/// # Errors
/// The bytes are not a `ChainHead`, or it names no page.
pub fn decode_chain_head(bytes: &[u8]) -> Result<PagePointer, CodecError> {
    let reader = unpack(bytes)?;
    let r = reader
        .get_root::<chain_capnp::chain_head::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    if !r.has_newest() {
        return Err(CodecError::Deserialization(
            "chain head names no page".into(),
        ));
    }
    read_pointer(r.get_newest().map_err(|e| capnp_err(&e))?)
}

/// Participant record subkey `RETIRE_THROUGH`.
#[must_use]
pub fn encode_chain_retire(through_seq: u64) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    builder
        .init_root::<chain_capnp::chain_retire::Builder<'_>>()
        .set_through_seq(through_seq);
    pack(&builder)
}

/// # Errors
/// The bytes are not a `ChainRetire`.
pub fn decode_chain_retire(bytes: &[u8]) -> Result<u64, CodecError> {
    let reader = unpack(bytes)?;
    let r = reader
        .get_root::<chain_capnp::chain_retire::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    Ok(r.get_through_seq())
}

/// A page's only subkey.
#[must_use]
pub fn encode_chain_page(page: &ChainPage) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<chain_capnp::chain_page::Builder<'_>>();
        if let Some(prev) = &page.prev {
            set_pointer(root.reborrow().init_prev(), prev);
        }
        root.set_first_seq(page.first_seq);
        let len = u32::try_from(page.entries.len()).unwrap_or(u32::MAX);
        let mut list = root.init_entries(len);
        for (i, entry) in (0..len).zip(&page.entries) {
            list.set(i, entry);
        }
    }
    pack(&builder)
}

/// # Errors
/// The bytes are not a `ChainPage`.
pub fn decode_chain_page(bytes: &[u8]) -> Result<ChainPage, CodecError> {
    let reader = unpack(bytes)?;
    let r = reader
        .get_root::<chain_capnp::chain_page::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let prev = if r.has_prev() {
        Some(read_pointer(r.get_prev().map_err(|e| capnp_err(&e))?)?)
    } else {
        None
    };
    let list = r.get_entries().map_err(|e| capnp_err(&e))?;
    let entries = list.iter().map(data).collect::<Result<Vec<_>, _>>()?;
    Ok(ChainPage {
        prev,
        first_seq: r.get_first_seq(),
        entries,
    })
}

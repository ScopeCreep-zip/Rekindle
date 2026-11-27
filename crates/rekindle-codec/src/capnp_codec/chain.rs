//! The append-only chain's values (plan E2.5, primitive N2;
//! `schemas/chain.capnp`). This is the wire shape only: whether a page fits
//! its pointer and the page before it is `rekindle_records::chain`'s check.

use super::direct::{data, fixed};
use super::{bounds, capnp_err, pack, text_to_string, unpack_bounded, CodecError};
use crate::chain_capnp;

/// Where a page sits: its record key and the sequence numbers it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagePointer {
    pub page_key: String,
    pub first_seq: u64,
    pub last_seq: u64,
}

/// A chain's head value: the newest page and the retire mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHead {
    pub newest: PagePointer,
    /// Every entry at or below it is retired (0: none).
    pub retired_through: u64,
}

/// A locator on a device's old GOV_HEAD. Governance verifies the named op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRetirementPointer {
    pub op_hash: [u8; 32],
    pub successor_key: [u8; 32],
    pub successor_record: String,
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

/// Participant record subkey `HEAD`: the newest page and the retire mark.
pub fn encode_chain_head(head: &ChainHead) -> Result<Vec<u8>, CodecError> {
    encode_head_value(Some(head), None)
}

/// Encode the head and its optional governance retirement locator in one value.
pub fn encode_head_value(
    head: Option<&ChainHead>,
    retirement: Option<&DeviceRetirementPointer>,
) -> Result<Vec<u8>, CodecError> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<chain_capnp::chain_head::Builder<'_>>();
        if let Some(head) = head {
            root.set_retired_through(head.retired_through);
            set_pointer(root.reborrow().init_newest(), &head.newest);
        }
        if let Some(pointer) = retirement {
            let mut r = root.init_device_retirement();
            r.set_op_hash(&pointer.op_hash);
            r.set_successor_key(&pointer.successor_key);
            r.set_successor_record(pointer.successor_record.as_str());
        }
    }
    pack(&builder)
}

/// The head a chain's writer puts on the network when it creates the
/// record, before its key leaves: no page yet and nothing retired. Veilid
/// creates a record locally and writes nothing to the network (0.5.8
/// `storage_manager/create_record.rs`), so a reader given the key of a record
/// nobody has written finds no record at all; this value is the first write.
pub fn encode_empty_chain_head() -> Result<Vec<u8>, CodecError> {
    encode_head_value(None, None)
}

/// `None`: the empty head ([`encode_empty_chain_head`]), a chain with nothing
/// appended.
///
/// # Errors
/// The bytes are not a `ChainHead`, or a head naming no page retires
/// something.
pub fn decode_chain_head(bytes: &[u8]) -> Result<Option<ChainHead>, CodecError> {
    decode_head(bytes, false).map(|(head, _)| head)
}

/// Decode the op-log head and its retirement locator.
pub fn decode_head_value(
    bytes: &[u8],
) -> Result<(Option<ChainHead>, Option<DeviceRetirementPointer>), CodecError> {
    decode_head(bytes, true)
}

fn decode_head(
    bytes: &[u8],
    read_retirement: bool,
) -> Result<(Option<ChainHead>, Option<DeviceRetirementPointer>), CodecError> {
    let reader = unpack_bounded(bytes, bounds::CHAIN_HEAD_BOUND)?;
    let r = reader
        .get_root::<chain_capnp::chain_head::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let head = if r.has_newest() {
        Some(ChainHead {
            newest: read_pointer(r.get_newest().map_err(|e| capnp_err(&e))?)?,
            retired_through: r.get_retired_through(),
        })
    } else {
        if r.get_retired_through() != 0 {
            return Err(CodecError::Deserialization(
                "chain head names no page yet retires entries".into(),
            ));
        }
        None
    };
    let retirement = if read_retirement && r.has_device_retirement() {
        let pointer = r.get_device_retirement().map_err(|e| capnp_err(&e))?;
        Some(DeviceRetirementPointer {
            op_hash: fixed(pointer.get_op_hash(), "retirement op hash")?,
            successor_key: fixed(pointer.get_successor_key(), "successor key")?,
            successor_record: text_to_string(
                pointer.get_successor_record().map_err(|e| capnp_err(&e))?,
            )?,
        })
    } else {
        None
    };
    Ok((head, retirement))
}

/// A page's only subkey.
pub fn encode_chain_page(page: &ChainPage) -> Result<Vec<u8>, CodecError> {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<chain_capnp::chain_page::Builder<'_>>();
        if let Some(prev) = &page.prev {
            set_pointer(root.reborrow().init_prev(), prev);
        }
        root.set_first_seq(page.first_seq);
        let len = crate::capnp_codec::wire_len(page.entries.len())?;
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
    let reader = unpack_bounded(bytes, bounds::CHAIN_PAGE_BOUND)?;
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

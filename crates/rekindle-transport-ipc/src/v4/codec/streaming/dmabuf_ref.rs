//! DmaBufRef encode/decode. Fixed 65 bytes on wire.

use crate::v4::streaming::dmabuf::{DmaBufPlane, DmaBufRef};
use super::CodecError;

pub const WIRE_SIZE: usize = DmaBufRef::WIRE_SIZE;

/// Encode a DmaBufRef into a fixed-size buffer.
pub fn encode(dmabuf: &DmaBufRef, buf: &mut [u8; WIRE_SIZE]) {
    buf[0..8].copy_from_slice(&dmabuf.pts.to_le_bytes());
    buf[8..12].copy_from_slice(&dmabuf.width.to_le_bytes());
    buf[12..16].copy_from_slice(&dmabuf.height.to_le_bytes());
    buf[16..20].copy_from_slice(&dmabuf.fourcc.to_le_bytes());
    buf[20..28].copy_from_slice(&dmabuf.modifier.to_le_bytes());
    buf[28] = dmabuf.num_planes;
    buf[29..32].copy_from_slice(&[0u8; 3]); // pad
    for i in 0..4 {
        let base = 32 + i * 8;
        buf[base..base + 4].copy_from_slice(&dmabuf.planes[i].stride.to_le_bytes());
        buf[base + 4..base + 8].copy_from_slice(&dmabuf.planes[i].offset.to_le_bytes());
    }
    buf[64] = dmabuf.payload_id_hint;
}

/// Decode a DmaBufRef from `buf`.
pub fn decode(buf: &[u8]) -> Result<DmaBufRef, CodecError> {
    if buf.len() < WIRE_SIZE {
        return Err(CodecError::TooShort {
            expected: WIRE_SIZE,
            got: buf.len(),
        });
    }

    let mut planes = [DmaBufPlane::default(); 4];
    for i in 0..4 {
        let base = 32 + i * 8;
        planes[i] = DmaBufPlane {
            stride: u32::from_le_bytes(buf[base..base + 4].try_into().unwrap()),
            offset: u32::from_le_bytes(buf[base + 4..base + 8].try_into().unwrap()),
        };
    }

    Ok(DmaBufRef {
        pts: u64::from_le_bytes(buf[0..8].try_into().unwrap()),
        width: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
        height: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
        fourcc: u32::from_le_bytes(buf[16..20].try_into().unwrap()),
        modifier: u64::from_le_bytes(buf[20..28].try_into().unwrap()),
        num_planes: buf[28],
        _pad: [0u8; 3],
        planes,
        payload_id_hint: buf[64],
    })
}

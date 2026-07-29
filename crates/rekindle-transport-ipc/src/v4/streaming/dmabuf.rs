//! DMA-BUF pass-through types for GPU-to-GPU frame transfer (Tier 3).
//!
//! These types describe GPU buffer metadata for the control plane.
//! The actual dmabuf fd is passed via sidechannel SCM_RIGHTS.
//! The DmaBufRef wire message carries the metadata needed to import
//! the buffer into the receiver's GPU context.
//!
//! Tier 3 implementation (GPU import/export) is out of scope for this
//! crate — it lives in SpiritStream's capture/encode backends. This
//! module provides the wire types only. See `codec::streaming::dmabuf_ref`
//! for encoding.

/// Per-plane layout descriptor for multi-planar pixel formats.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DmaBufPlane {
    pub stride: u32,
    pub offset: u32,
}

/// DMA-BUF reference sent over the encrypted control channel.
/// The actual fd is sent via sidechannel SCM_RIGHTS, tagged with
/// `payload_id_hint` for correlation.
///
/// 65 bytes on wire. See `codec::streaming::dmabuf_ref` for encoding.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufRef {
    /// Presentation timestamp (nanoseconds, monotonic clock).
    pub pts: u64,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// DRM fourcc format code (e.g., DRM_FORMAT_NV12 = 0x3231564E).
    pub fourcc: u32,
    /// DRM format modifier (e.g., I915_FORMAT_MOD_Y_TILED).
    pub modifier: u64,
    /// Number of planes (1 for packed, 2 for NV12, 3 for YUV420P).
    pub num_planes: u8,
    /// Padding for alignment.
    pub _pad: [u8; 3],
    /// Per-plane stride and offset. Only `planes[0..num_planes]` are valid.
    pub planes: [DmaBufPlane; 4],
    /// Correlates with the sidechannel FdTag.payload_id for fd matching.
    pub payload_id_hint: u8,
}

impl DmaBufRef {
    pub const WIRE_SIZE: usize = 65;
}

impl Default for DmaBufRef {
    fn default() -> Self {
        Self {
            pts: 0,
            width: 0,
            height: 0,
            fourcc: 0,
            modifier: 0,
            num_planes: 0,
            _pad: [0; 3],
            planes: [DmaBufPlane::default(); 4],
            payload_id_hint: 0,
        }
    }
}

// Compile-time verification that the struct can hold the wire representation.
const _: () = assert!(
    core::mem::size_of::<DmaBufRef>() >= DmaBufRef::WIRE_SIZE,
    "DmaBufRef struct must be at least WIRE_SIZE bytes"
);

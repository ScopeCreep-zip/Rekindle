//! DmaBufRef type tests — validates the data structure invariants.
//! No GPU or DRM operations — just the type layout, encoding, and
//! the contract that Tier 3 handler code will rely on.

use rekindle_transport_ipc::v4::streaming::dmabuf::{DmaBufRef, DmaBufPlane};

// ── Struct layout ───────────────────────────────────────────────

#[test]
fn dmabuf_plane_is_8_bytes() {
    assert_eq!(std::mem::size_of::<DmaBufPlane>(), 8);
}

#[test]
fn dmabuf_ref_contains_4_planes() {
    let r = DmaBufRef {
        pts: 0,
        width: 0,
        height: 0,
        fourcc: 0,
        modifier: 0,
        num_planes: 0,
        _pad: [0; 3],
        planes: [DmaBufPlane { stride: 0, offset: 0 }; 4],
        payload_id_hint: 0,
    };
    assert_eq!(r.planes.len(), 4);
}

// ── NV12 format constants ───────────────────────────────────────

#[test]
fn nv12_fourcc_value() {
    // NV12 = 'N' 'V' '1' '2' = 0x3231564E in little-endian DRM convention
    let fourcc = u32::from_le_bytes([b'N', b'V', b'1', b'2']);
    assert_eq!(fourcc, 0x3231564E);
}

#[test]
fn nv12_4k_plane_layout() {
    let width: u32 = 3840;
    let height: u32 = 2160;

    // NV12: Y plane at offset 0, stride = width
    // UV plane at offset = width * height, stride = width
    let y_offset = 0u32;
    let y_stride = width;
    let uv_offset = width * height; // 8,294,400
    let uv_stride = width;

    assert_eq!(uv_offset, 8_294_400);

    let planes = [
        DmaBufPlane { stride: y_stride, offset: y_offset },
        DmaBufPlane { stride: uv_stride, offset: uv_offset },
        DmaBufPlane { stride: 0, offset: 0 },
        DmaBufPlane { stride: 0, offset: 0 },
    ];

    let r = DmaBufRef {
        pts: 0,
        width,
        height,
        fourcc: 0x3231564E,
        modifier: 0,
        num_planes: 2,
        _pad: [0; 3],
        planes,
        payload_id_hint: 0,
    };

    assert_eq!(r.num_planes, 2);
    assert_eq!(r.planes[0].stride, 3840);
    assert_eq!(r.planes[0].offset, 0);
    assert_eq!(r.planes[1].stride, 3840);
    assert_eq!(r.planes[1].offset, 8_294_400);
}

// ── P010 (10-bit HDR) plane layout ──────────────────────────────

#[test]
fn p010_4k_plane_layout() {
    let width: u32 = 3840;
    let height: u32 = 2160;

    // P010: 16 bits per Y sample, 16 bits per UV sample
    // Y plane stride = width * 2
    // UV plane at offset = width * 2 * height
    let y_stride = width * 2;
    let uv_offset = y_stride * height;
    let uv_stride = width * 2;

    assert_eq!(y_stride, 7680);
    assert_eq!(uv_offset, 16_588_800);

    let r = DmaBufRef {
        pts: 0,
        width,
        height,
        fourcc: 0x30313050, // P010
        modifier: 0,
        num_planes: 2,
        _pad: [0; 3],
        planes: [
            DmaBufPlane { stride: y_stride, offset: 0 },
            DmaBufPlane { stride: uv_stride, offset: uv_offset },
            DmaBufPlane { stride: 0, offset: 0 },
            DmaBufPlane { stride: 0, offset: 0 },
        ],
        payload_id_hint: 0,
    };

    assert_eq!(r.num_planes, 2);
}

// ── BGRA (32-bit) single plane ──────────────────────────────────

#[test]
fn bgra_4k_single_plane() {
    let width: u32 = 3840;
    let height: u32 = 2160;

    let r = DmaBufRef {
        pts: 0,
        width,
        height,
        fourcc: 0x34324241, // AB24
        modifier: 0,
        num_planes: 1,
        _pad: [0; 3],
        planes: [
            DmaBufPlane { stride: width * 4, offset: 0 },
            DmaBufPlane { stride: 0, offset: 0 },
            DmaBufPlane { stride: 0, offset: 0 },
            DmaBufPlane { stride: 0, offset: 0 },
        ],
        payload_id_hint: 0,
    };

    assert_eq!(r.planes[0].stride, 15360);
    assert_eq!(r.num_planes, 1);
}

// ── payload_id_hint covers full u8 range ────────────────────────

#[test]
fn payload_id_hint_full_range() {
    for id in [0u8, 1, 127, 128, 255] {
        let r = DmaBufRef {
            pts: 0,
            width: 0,
            height: 0,
            fourcc: 0,
            modifier: 0,
            num_planes: 0,
            _pad: [0; 3],
            planes: [DmaBufPlane { stride: 0, offset: 0 }; 4],
            payload_id_hint: id,
        };
        assert_eq!(r.payload_id_hint, id);
    }
}

// ── modifier field carries DRM format modifiers ─────────────────

#[test]
fn modifier_field_carries_tiled_format() {
    // I915_FORMAT_MOD_Y_TILED = 0x0100000000000002
    let modifier: u64 = 0x0100_0000_0000_0002;

    let r = DmaBufRef {
        pts: 0,
        width: 1920,
        height: 1080,
        fourcc: 0x3231564E,
        modifier,
        num_planes: 2,
        _pad: [0; 3],
        planes: [DmaBufPlane { stride: 0, offset: 0 }; 4],
        payload_id_hint: 0,
    };

    assert_eq!(r.modifier, 0x0100_0000_0000_0002);
}

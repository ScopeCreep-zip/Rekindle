//! Wire format roundtrip tests for all v4 streaming frame types.
//!
//! Each test:
//! 1. Constructs a frame with known field values
//! 2. Encodes to bytes
//! 3. Verifies encoded byte count matches spec
//! 4. Decodes from bytes
//! 5. Asserts EVERY field individually (not struct equality — catches silent default fields)
//! 6. Tests boundary values (0, MAX, typical production values)

use rekindle_transport_ipc::v4::streaming::shared_arena::{SharedMemRef, SlotRelease};
use rekindle_transport_ipc::v4::streaming::dmabuf::{DmaBufRef, DmaBufPlane};
use rekindle_transport_ipc::v4::codec::streaming::{
    arena_write, slot_release, arena_setup, arena_ack, dmabuf_ref,
};

// ── SharedMemRef ────────────────────────────────────────────────

#[test]
fn shared_mem_ref_no_integrity_is_15_bytes() {
    assert_eq!(arena_write::WIRE_SIZE_NO_DIGEST, 15);
    assert_eq!(arena_write::wire_size(false), 15);
}

#[test]
fn shared_mem_ref_with_integrity_is_47_bytes() {
    assert_eq!(arena_write::WIRE_SIZE_WITH_DIGEST, 47);
    assert_eq!(arena_write::wire_size(true), 47);
}

#[test]
fn shared_mem_ref_roundtrip_no_integrity() {
    let original = SharedMemRef {
        arena_id: 3, slot: 7, generation: 42,
        offset: 0, length: 12_441_600, digest: None,
    };
    let mut buf = [0u8; 47];
    arena_write::encode(&original, &mut buf, false);
    let decoded = arena_write::decode(&buf[..15], false).unwrap();

    assert_eq!(decoded.arena_id, 3, "arena_id mismatch");
    assert_eq!(decoded.slot, 7, "slot mismatch");
    assert_eq!(decoded.generation, 42, "generation mismatch");
    assert_eq!(decoded.offset, 0, "offset mismatch");
    assert_eq!(decoded.length, 12_441_600, "length mismatch");
    assert_eq!(decoded.digest, None, "digest must be None without integrity");
}

#[test]
fn shared_mem_ref_roundtrip_with_integrity() {
    let digest = [0xAA; 32];
    let original = SharedMemRef {
        arena_id: 0, slot: 7, generation: 42,
        offset: 0, length: 12_441_600, digest: Some(digest),
    };
    let mut buf = [0u8; 47];
    arena_write::encode(&original, &mut buf, true);
    let decoded = arena_write::decode(&buf, true).unwrap();

    assert_eq!(decoded.arena_id, 0, "arena_id mismatch");
    assert_eq!(decoded.slot, 7, "slot mismatch");
    assert_eq!(decoded.generation, 42, "generation mismatch");
    assert_eq!(decoded.offset, 0, "offset mismatch");
    assert_eq!(decoded.length, 12_441_600, "length mismatch");
    assert_eq!(decoded.digest, Some(digest), "digest mismatch");
}

#[test]
fn shared_mem_ref_max_field_values() {
    let original = SharedMemRef {
        arena_id: u8::MAX, slot: u16::MAX, generation: u32::MAX,
        offset: u32::MAX, length: u32::MAX, digest: Some([0xFF; 32]),
    };
    let mut buf = [0u8; 47];
    arena_write::encode(&original, &mut buf, true);
    let decoded = arena_write::decode(&buf, true).unwrap();

    // Field-by-field — catches silent new fields with Default values
    assert_eq!(decoded.arena_id, u8::MAX, "arena_id MAX");
    assert_eq!(decoded.slot, u16::MAX, "slot MAX");
    assert_eq!(decoded.generation, u32::MAX, "generation MAX");
    assert_eq!(decoded.offset, u32::MAX, "offset MAX");
    assert_eq!(decoded.length, u32::MAX, "length MAX");
    assert_eq!(decoded.digest, Some([0xFF; 32]), "digest MAX");
    // Structural equality as a safety net — if a field is added and
    // not covered above, this catches the mismatch
    assert_eq!(decoded, original, "structural equality after field-by-field");
}

#[test]
fn shared_mem_ref_zero_field_values() {
    let original = SharedMemRef {
        arena_id: 0, slot: 0, generation: 0,
        offset: 0, length: 0, digest: Some([0x00; 32]),
    };
    let mut buf = [0u8; 47];
    arena_write::encode(&original, &mut buf, true);
    let decoded = arena_write::decode(&buf, true).unwrap();

    assert_eq!(decoded.arena_id, 0);
    assert_eq!(decoded.slot, 0);
    assert_eq!(decoded.generation, 0);
    assert_eq!(decoded.length, 0);
}

#[test]
fn shared_mem_ref_decode_rejects_short_buffer() {
    assert!(arena_write::decode(&[0u8; 14], false).is_err(), "14 bytes must fail for 15-byte format");
    assert!(arena_write::decode(&[0u8; 46], true).is_err(), "46 bytes must fail for 47-byte format");
}

// ── SlotRelease ─────────────────────────────────────────────────

#[test]
fn slot_release_is_7_bytes() {
    assert_eq!(slot_release::WIRE_SIZE, 7);
}

#[test]
fn slot_release_roundtrip() {
    let original = SlotRelease { arena_id: 2, slot: 7, generation: 42 };
    let mut buf = [0u8; 7];
    slot_release::encode(&original, &mut buf);
    let decoded = slot_release::decode(&buf).unwrap();

    assert_eq!(decoded.arena_id, 2, "arena_id mismatch");
    assert_eq!(decoded.slot, 7, "slot mismatch");
    assert_eq!(decoded.generation, 42, "generation mismatch");
}

#[test]
fn slot_release_decode_rejects_short_buffer() {
    assert!(slot_release::decode(&[0u8; 6]).is_err());
}

// ── ArenaSetup ──────────────────────────────────────────────────

#[test]
fn arena_setup_is_10_bytes() {
    assert_eq!(arena_setup::WIRE_SIZE, 10);
}

#[test]
fn arena_setup_roundtrip() {
    let original = arena_setup::ArenaSetupPayload {
        slot_size: 16_777_216, slot_count: 8, integrity: 1,
    };
    let buf = arena_setup::encode(&original);
    assert_eq!(buf.len(), 10);
    let decoded = arena_setup::decode(&buf).unwrap();

    assert_eq!(decoded.slot_size, 16_777_216, "slot_size mismatch");
    assert_eq!(decoded.slot_count, 8, "slot_count mismatch");
    assert_eq!(decoded.integrity, 1, "integrity mismatch");
}

#[test]
fn arena_setup_integrity_zero() {
    let original = arena_setup::ArenaSetupPayload {
        slot_size: 4096, slot_count: 4, integrity: 0,
    };
    let buf = arena_setup::encode(&original);
    let decoded = arena_setup::decode(&buf).unwrap();
    assert_eq!(decoded.integrity, 0);
}

#[test]
fn arena_setup_decode_rejects_short_buffer() {
    assert!(arena_setup::decode(&[0u8; 9]).is_err());
}

// ── ArenaAck ────────────────────────────────────────────────────

#[test]
fn arena_ack_is_1_byte() {
    assert_eq!(arena_ack::WIRE_SIZE, 1);
}

#[test]
fn arena_ack_ok_roundtrip() {
    let buf = arena_ack::encode(arena_ack::STATUS_OK);
    assert_eq!(buf.len(), 1);
    assert_eq!(buf[0], 0);
    let decoded = arena_ack::decode(&buf).unwrap();
    assert_eq!(decoded, arena_ack::STATUS_OK);
}

#[test]
fn arena_ack_rejected_roundtrip() {
    let buf = arena_ack::encode(arena_ack::STATUS_REJECTED);
    assert_eq!(buf[0], 1);
    let decoded = arena_ack::decode(&buf).unwrap();
    assert_eq!(decoded, arena_ack::STATUS_REJECTED);
}

#[test]
fn arena_ack_decode_rejects_empty_buffer() {
    assert!(arena_ack::decode(&[]).is_err());
}

// ── DmaBufRef ───────────────────────────────────────────────────

#[test]
fn dmabuf_ref_is_65_bytes() {
    assert_eq!(dmabuf_ref::WIRE_SIZE, 65);
    assert_eq!(DmaBufRef::WIRE_SIZE, 65);
}

#[test]
fn dmabuf_ref_nv12_4k_roundtrip() {
    let original = DmaBufRef {
        pts: 1_000_000_000, width: 3840, height: 2160,
        fourcc: 0x3231564E, modifier: 0, num_planes: 2,
        _pad: [0; 3],
        planes: [
            DmaBufPlane { stride: 3840, offset: 0 },
            DmaBufPlane { stride: 3840, offset: 3840 * 2160 },
            DmaBufPlane::default(),
            DmaBufPlane::default(),
        ],
        payload_id_hint: 42,
    };
    let mut buf = [0u8; 65];
    dmabuf_ref::encode(&original, &mut buf);
    let decoded = dmabuf_ref::decode(&buf).unwrap();

    assert_eq!(decoded.pts, 1_000_000_000, "pts mismatch");
    assert_eq!(decoded.width, 3840, "width mismatch");
    assert_eq!(decoded.height, 2160, "height mismatch");
    assert_eq!(decoded.fourcc, 0x3231564E, "fourcc mismatch");
    assert_eq!(decoded.modifier, 0, "modifier mismatch");
    assert_eq!(decoded.num_planes, 2, "num_planes mismatch");
    assert_eq!(decoded.planes[0].stride, 3840, "plane0 stride mismatch");
    assert_eq!(decoded.planes[0].offset, 0, "plane0 offset mismatch");
    assert_eq!(decoded.planes[1].stride, 3840, "plane1 stride mismatch");
    assert_eq!(decoded.planes[1].offset, 3840 * 2160, "plane1 offset mismatch");
    assert_eq!(decoded.payload_id_hint, 42, "payload_id_hint mismatch");
}

#[test]
fn dmabuf_ref_tiled_bgra_roundtrip() {
    let original = DmaBufRef {
        pts: u64::MAX, width: 1920, height: 1080,
        fourcc: 0x34324241,
        modifier: 0x0100_0000_0000_0001, // I915_FORMAT_MOD_X_TILED
        num_planes: 1, _pad: [0; 3],
        planes: [
            DmaBufPlane { stride: 7680, offset: 0 },
            DmaBufPlane::default(),
            DmaBufPlane::default(),
            DmaBufPlane::default(),
        ],
        payload_id_hint: 0,
    };
    let mut buf = [0u8; 65];
    dmabuf_ref::encode(&original, &mut buf);
    let decoded = dmabuf_ref::decode(&buf).unwrap();

    assert_eq!(decoded.pts, u64::MAX, "pts MAX mismatch");
    assert_eq!(decoded.fourcc, 0x34324241, "fourcc mismatch");
    assert_eq!(decoded.modifier, 0x0100_0000_0000_0001, "modifier mismatch");
    assert_eq!(decoded.num_planes, 1, "num_planes mismatch");
    assert_eq!(decoded.planes[0].stride, 7680, "stride mismatch");
}

#[test]
fn dmabuf_ref_decode_rejects_short_buffer() {
    assert!(dmabuf_ref::decode(&[0u8; 64]).is_err(), "64 bytes must fail for 65-byte format");
}

use std::collections::HashSet;
use rekindle_transport_ipc::v3::wire::frame_kind::*;

#[test]
fn all_channel_kinds_have_distinct_values() {
    let mut seen = HashSet::new();
    for &k in ChannelKind::all_variants() {
        let v = k as u8;
        assert!(seen.insert(v), "duplicate ChannelKind value: 0x{v:02x}");
    }
}

#[test]
fn all_stream_kinds_have_distinct_values() {
    let mut seen = HashSet::new();
    for &k in StreamKind::all_variants() {
        let v = k as u8;
        assert!(seen.insert(v), "duplicate StreamKind value: 0x{v:02x}");
    }
}

#[test]
fn all_datagram_kinds_have_distinct_values() {
    let mut seen = HashSet::new();
    for &k in DatagramKind::all_variants() {
        let v = k as u8;
        assert!(seen.insert(v), "duplicate DatagramKind value: 0x{v:02x}");
    }
}

#[test]
fn all_audit_kinds_have_distinct_values() {
    let mut seen = HashSet::new();
    for &k in AuditKind::all_variants() {
        let v = k as u8;
        assert!(seen.insert(v), "duplicate AuditKind value: 0x{v:02x}");
    }
}

#[test]
fn all_handoff_kinds_have_distinct_values() {
    let mut seen = HashSet::new();
    for &k in HandoffKind::all_variants() {
        let v = k as u8;
        assert!(seen.insert(v), "duplicate HandoffKind value: 0x{v:02x}");
    }
}

// Guard: all_variants() returns every variant the enum defines.
// If someone adds a variant but forgets to add it to all_variants(),
// this test catches the mismatch by verifying the count matches
// the TryFrom range.

#[test]
fn channel_kind_all_variants_is_exhaustive() {
    let variants = ChannelKind::all_variants();
    // Every value that TryFrom accepts must be in all_variants
    for v in 0x00..=0xFF_u8 {
        if ChannelKind::try_from(v).is_ok() {
            assert!(
                variants.iter().any(|&k| k as u8 == v),
                "ChannelKind::try_from(0x{v:02x}) succeeds but \
                 0x{v:02x} is not in all_variants()"
            );
        }
    }
    // Every value in all_variants must be accepted by TryFrom
    for &k in variants {
        assert!(
            ChannelKind::try_from(k as u8).is_ok(),
            "ChannelKind::{k:?} is in all_variants() but \
             TryFrom rejects its wire value"
        );
    }
}

#[test]
fn stream_kind_all_variants_is_exhaustive() {
    let variants = StreamKind::all_variants();
    for v in 0x00..=0xFF_u8 {
        if StreamKind::try_from(v).is_ok() {
            assert!(
                variants.iter().any(|&k| k as u8 == v),
                "StreamKind::try_from(0x{v:02x}) succeeds but \
                 0x{v:02x} is not in all_variants()"
            );
        }
    }
    for &k in variants {
        assert!(StreamKind::try_from(k as u8).is_ok());
    }
}

#[test]
fn datagram_kind_all_variants_is_exhaustive() {
    let variants = DatagramKind::all_variants();
    for v in 0x00..=0xFF_u8 {
        if DatagramKind::try_from(v).is_ok() {
            assert!(
                variants.iter().any(|&k| k as u8 == v),
                "DatagramKind::try_from(0x{v:02x}) succeeds but \
                 0x{v:02x} is not in all_variants()"
            );
        }
    }
    for &k in variants {
        assert!(DatagramKind::try_from(k as u8).is_ok());
    }
}

#[test]
fn audit_kind_all_variants_is_exhaustive() {
    let variants = AuditKind::all_variants();
    for v in 0x00..=0xFF_u8 {
        if AuditKind::try_from(v).is_ok() {
            assert!(
                variants.iter().any(|&k| k as u8 == v),
                "AuditKind::try_from(0x{v:02x}) succeeds but \
                 0x{v:02x} is not in all_variants()"
            );
        }
    }
    for &k in variants {
        assert!(AuditKind::try_from(k as u8).is_ok());
    }
}

#[test]
fn handoff_kind_all_variants_is_exhaustive() {
    let variants = HandoffKind::all_variants();
    for v in 0x00..=0xFF_u8 {
        if HandoffKind::try_from(v).is_ok() {
            assert!(
                variants.iter().any(|&k| k as u8 == v),
                "HandoffKind::try_from(0x{v:02x}) succeeds but \
                 0x{v:02x} is not in all_variants()"
            );
        }
    }
    for &k in variants {
        assert!(HandoffKind::try_from(k as u8).is_ok());
    }
}

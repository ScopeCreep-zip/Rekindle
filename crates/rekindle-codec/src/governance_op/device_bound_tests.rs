//! Lead decision (40), plan E3.2 S7d-keys K3b: [`MAX_DEVICE_SET_ELEMENTS`]
//! is exactly what fits its largest carrier, one `DeviceReEnrol` op
//! (S7d-keys.10), in one member-record subkey, every other field at its
//! largest encoded size.

use rekindle_types::governance::{
    DeviceEndorsement, DeviceReEnrol, DeviceRetired, DeviceSetElement, DeviceSetElementKind,
    GovernanceEntry, GovernanceOp, OpHash, OpRef, MAX_DEVICE_SET_ELEMENTS,
};
use rekindle_types::id::PseudonymKey;

use super::{
    decode_op, encode_op, op_unsigned_bytes, GOVERNANCE_OP_MAX_BYTES, GOVERNANCE_OP_MAX_DEPS,
};

/// Bytes that packing cannot shrink: packed framing elides only zero
/// bytes, so every byte here is non-zero.
fn noise<const N: usize>(seed: usize) -> [u8; N] {
    std::array::from_fn(|j| u8::try_from((seed * 131 + j * 29) % 255 + 1).unwrap())
}

/// A record key's text at full length (`VLD0:` key `:` encryption key).
fn record_text(c: char) -> String {
    format!(
        "VLD0:{}:{}",
        c.to_string().repeat(43),
        c.to_string().repeat(43)
    )
}

/// `n` distinct element ids, strictly ascending, no zero byte.
pub(super) fn elements(n: usize) -> Vec<DeviceSetElement> {
    (0..n)
        .map(|i| {
            let mut key: [u8; 32] = noise(i);
            key[0] = u8::try_from(i / 255 + 1).unwrap();
            key[1] = u8::try_from(i % 255 + 1).unwrap();
            DeviceSetElement::new(DeviceSetElementKind::Retirement, key, u64::MAX - 1)
        })
        .collect()
}

/// The largest `DeviceKey` op with `n` element ids: the full deps cap,
/// full-length texts and no zero byte anywhere it can be avoided.
fn worst_case_device_op(n: usize) -> GovernanceOp {
    worst_case_op(n, false)
}

/// The largest `DeviceReEnrol` op with `n` element ids: the
/// [`worst_case_device_op`] endorsement with a retirement beside it.
fn worst_case_re_enrol_op(n: usize) -> GovernanceOp {
    worst_case_op(n, true)
}

fn worst_case_op(n: usize, re_enrol: bool) -> GovernanceOp {
    let extends_elements = elements(n);
    let mut op = GovernanceOp {
        device_key: noise(1),
        author: PseudonymKey(noise(1)),
        seq: u64::MAX,
        prev: Some(OpHash(noise(2))),
        deps: (0..GOVERNANCE_OP_MAX_DEPS)
            .map(|i| OpRef {
                op_hash: OpHash(noise(10 + i)),
                device_key: noise(100 + i),
                author: PseudonymKey(noise(100 + i)),
                author_record: record_text('r'),
                author_seq: u64::MAX - i as u64,
            })
            .collect(),
        issued_at: u64::MAX,
        body: GovernanceEntry::DeviceKey(DeviceEndorsement {
            device_key: noise(3),
            incarnation: u64::MAX,
            extends_set: noise(4),
            extends_elements,
            device_record: record_text('d'),
            device_record_owner: format!("VLD0:{}", "o".repeat(43)),
            fork_evidence: noise(5),
            endorsement_sig: noise::<64>(6).to_vec(),
        }),
        sig: noise(7),
    };
    if re_enrol {
        let GovernanceEntry::DeviceKey(endorsement) = op.body.clone() else {
            unreachable!("built as a DeviceKey above");
        };
        op.body = GovernanceEntry::DeviceReEnrol(DeviceReEnrol {
            retired: DeviceRetired {
                device_key: noise(8),
                incarnation: u64::MAX - 1,
                last_seq: u64::MAX,
                retirement_sig: noise::<64>(9).to_vec(),
            },
            endorsement,
        });
    }
    op
}

fn endorsement(op: &GovernanceOp) -> &DeviceEndorsement {
    match &op.body {
        GovernanceEntry::DeviceKey(e) => e,
        GovernanceEntry::DeviceReEnrol(r) => &r.endorsement,
        other => panic!("not a device endorsement op: {other:?}"),
    }
}

/// At the bound the largest op of either arm seals within the subkey,
/// reads back and is well formed; one element more makes the largest
/// carrier, the `DeviceReEnrol`, fit neither, and no endorsement well
/// formed.
#[test]
fn the_extended_set_bound_is_what_fits_one_op() {
    for at in [
        worst_case_device_op(MAX_DEVICE_SET_ELEMENTS),
        worst_case_re_enrol_op(MAX_DEVICE_SET_ELEMENTS),
    ] {
        let bytes = encode_op(&at).unwrap();
        assert!(bytes.len() <= GOVERNANCE_OP_MAX_BYTES);
        assert_eq!(decode_op(&bytes).unwrap().op, at);
        assert!(endorsement(&at).well_formed());
    }
    let over = worst_case_re_enrol_op(MAX_DEVICE_SET_ELEMENTS + 1);
    assert!(encode_op(&over).is_err());
    assert!(!endorsement(&over).well_formed());
    assert!(!endorsement(&worst_case_device_op(MAX_DEVICE_SET_ELEMENTS + 1)).well_formed());
}

/// The derivation's terms (`MAX_DEVICE_SET_ELEMENTS`' doc) are the
/// encoding's: the rest of a `DeviceKey` op is 1,928 words, of a
/// `DeviceReEnrol` 1,946, each element 7.
#[test]
fn the_bounds_derivation_matches_the_encoding() {
    let words = |n| op_unsigned_bytes(&worst_case_device_op(n)).len() / 8;
    let re_enrol_words = |n| op_unsigned_bytes(&worst_case_re_enrol_op(n)).len() / 8;
    assert_eq!(words(0), 1_928);
    assert_eq!(re_enrol_words(0), 1_946);
    assert_eq!(words(2) - words(1), 7);
    assert_eq!(
        re_enrol_words(MAX_DEVICE_SET_ELEMENTS),
        1_946 + 7 * MAX_DEVICE_SET_ELEMENTS
    );
}

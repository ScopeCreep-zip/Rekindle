use super::*;

fn bound(record: &str) -> DeviceRecordKeys {
    DeviceRecordKeys {
        record: record.into(),
        owner: Zeroizing::new("K_M(p,d)".into()),
    }
}

/// A second device publishes its op on `M(p,d)` with `K_M(p,d)`.
#[test]
fn a_bound_record_is_written_with_its_own_owner() {
    let Ok(OpRecordOwner::Device(owner)) = op_record_owner("VLD0:mpd", Some(bound("VLD0:mpd")))
    else {
        panic!("M(p,d) is owned by K_M(p,d)");
    };
    assert_eq!(owner.as_str(), "K_M(p,d)");
}

/// The first device's entry binds no record: `M(p)` with `K_M(p)`.
#[test]
fn the_first_device_writes_with_the_member_owner() {
    assert!(matches!(
        op_record_owner("VLD0:mp", None),
        Ok(OpRecordOwner::Member)
    ));
}

/// A device bound to `M(p,d)` never writes `M(p)` with `K_M(p)`.
#[test]
fn a_bound_device_never_writes_another_record() {
    assert!(op_record_owner("VLD0:mp", Some(bound("VLD0:mpd"))).is_err());
}

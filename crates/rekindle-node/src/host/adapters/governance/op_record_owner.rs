//! The owner keypair that writes our op record (ADR 0022 rules 3, 4): a
//! device other than the member's first writes its ops to the `M(p,d)` its
//! key's endorsement binds, owned by `K_M(p,d)` from its device vault
//! entry (`DeviceOpKey{community}`); the first device's entry binds no
//! record, its op record is `M(p)`, owned by `K_M(p)` in the identity vault.

use rekindle_vault::typed::device_keys::DeviceRecordKeys;
use zeroize::Zeroizing;

use super::HostGovernance;

/// Who owns our op record.
pub(super) enum OpRecordOwner {
    /// `K_M(p,d)`, from the device vault.
    Device(Zeroizing<String>),
    /// `K_M(p)`, from the identity vault: the first device's `M(p)`.
    Member,
}

/// The owner of our op record `record`, given the record the device vault
/// entry binds: that entry's `K_M(p,d)` for its own record, `K_M(p)` when
/// the entry binds none (the first device). A device that binds `M(p,d)`
/// never writes another record, `M(p)` included.
///
/// # Errors
/// `record` is not the record the entry binds.
pub(super) fn op_record_owner(
    record: &str,
    bound: Option<DeviceRecordKeys>,
) -> Result<OpRecordOwner, String> {
    match bound {
        None => Ok(OpRecordOwner::Member),
        Some(keys) if keys.record == record => Ok(OpRecordOwner::Device(keys.owner)),
        Some(keys) => Err(format!(
            "{record} is not this device's op record ({})",
            keys.record
        )),
    }
}

impl HostGovernance {
    /// Our op record in `community_id` and its owner keypair: the record
    /// our device key's endorsement binds in our merged state
    /// (`device_signer::own_op_record`, the resolver the op write path
    /// takes), and that record's owner ([`Self::op_record_writer`]). The
    /// one resolution for a write of our op chain's frontier (`M`/3) that
    /// no queued op names a record for.
    ///
    /// # Errors
    /// As `own_op_record` (no membership, cert, device key or endorsement
    /// of it) or [`Self::op_record_writer`].
    pub(crate) fn own_op_record_writer(
        &self,
        community_id: &str,
    ) -> Result<(String, Zeroizing<String>), String> {
        let state = self.governance_state_impl(community_id).unwrap_or_default();
        let record =
            rekindle_governance_runtime::device_signer::own_op_record(self, community_id, &state)
                .map_err(|e| format!("own op record: {e}"))?;
        let writer = self.op_record_writer(community_id, &record)?;
        Ok((record, writer))
    }

    /// The owner keypair of our op record `record`
    /// ([`op_record_owner`]).
    pub(super) fn op_record_writer(
        &self,
        community_id: &str,
        record: &str,
    ) -> Result<Zeroizing<String>, String> {
        let bound = {
            let guard = self.ctx.device_vault.lock();
            let vault = guard.as_ref().ok_or("no identity is unlocked")?;
            rekindle_vault::typed::device_keys::load_device_record(vault, community_id)
                .map_err(|e| format!("device vault: {e}"))?
        };
        match op_record_owner(record, bound)? {
            OpRecordOwner::Device(owner) => Ok(owner),
            OpRecordOwner::Member => self.member_record_writer(community_id),
        }
    }
}

#[cfg(test)]
mod tests {
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
}

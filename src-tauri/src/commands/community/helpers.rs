use crate::state::SharedState;

pub(crate) use crate::state_helpers::hex_to_id_16;

pub(crate) fn u32_to_role_id(role_id: u32) -> rekindle_types::id::RoleId {
    let mut buf = [0u8; 16];
    buf[..4].copy_from_slice(&role_id.to_le_bytes());
    rekindle_types::id::RoleId(buf)
}

pub(crate) fn random_16_bytes() -> [u8; 16] {
    rekindle_utils::random::id_bytes_16()
}

pub(crate) fn random_nonce(bytes_len: usize) -> Vec<u8> {
    rekindle_utils::random::bytes(bytes_len)
}

pub(crate) fn require_permission(
    state: &SharedState,
    community_id: &str,
    required: u64,
) -> Result<(), String> {
    let communities = state.communities.read();
    let community = communities.get(community_id).ok_or("community not found")?;
    let gov = community
        .governance_state
        .as_ref()
        .ok_or("governance state not loaded for this community")?;
    let pseudo_hex = community
        .my_pseudonym_key
        .as_ref()
        .ok_or("no pseudonym key for this community")?;
    let pseudo_bytes: [u8; 32] = hex::decode(pseudo_hex)
        .map_err(|e| format!("invalid pseudonym hex: {e}"))?
        .try_into()
        .map_err(|_| "pseudonym must be 32 bytes")?;
    let pseudo = rekindle_types::id::PseudonymKey(pseudo_bytes);
    let perms = rekindle_governance::permissions::compute_permissions(
        &pseudo,
        None,
        gov,
        rekindle_utils::timestamp_secs(),
    );
    if rekindle_governance::permissions::has_all_capabilities(perms, required) {
        Ok(())
    } else {
        Err(format!("missing permission: {required:#x}"))
    }
}

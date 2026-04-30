use crate::state::SharedState;
use rekindle_protocol::dht::community::permissions_v2::Permissions;

pub(crate) fn hex_to_id_16(hex_str: &str) -> [u8; 16] {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 16])
}

pub(crate) fn hex_to_pseudo_32(hex_str: &str) -> [u8; 32] {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 32])
}

pub(crate) fn u32_to_role_id(role_id: u32) -> rekindle_types::id::RoleId {
    let mut buf = [0u8; 16];
    buf[..4].copy_from_slice(&role_id.to_le_bytes());
    rekindle_types::id::RoleId(buf)
}

pub(crate) fn random_16_bytes() -> [u8; 16] {
    use rand::RngCore;

    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

pub(crate) fn random_nonce(bytes_len: usize) -> Vec<u8> {
    use rand::RngCore;

    let mut nonce = vec![0u8; bytes_len];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    nonce
}

pub(crate) fn require_permission(
    state: &SharedState,
    community_id: &str,
    required: Permissions,
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
    if perms & rekindle_types::permissions::ADMINISTRATOR != 0
        || perms & required.bits() == required.bits()
    {
        Ok(())
    } else {
        Err(format!("missing permission: {required:?}"))
    }
}

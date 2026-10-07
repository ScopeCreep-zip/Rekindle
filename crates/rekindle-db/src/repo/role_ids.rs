//! Role-id sets as the `communities.my_role_ids` and
//! `community_members.role_ids` columns store them: a JSON array.

/// Encode a role-id set for storage.
pub(crate) fn encode(role_ids: &[u32]) -> rusqlite::Result<String> {
    serde_json::to_string(role_ids)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

/// Decode a stored role-id set (column `index` of the row being read).
pub(crate) fn decode(index: usize, json: &str) -> rusqlite::Result<Vec<u32>> {
    serde_json::from_str(json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}

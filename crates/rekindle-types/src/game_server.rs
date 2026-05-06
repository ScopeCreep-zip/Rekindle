//! Architecture §27 — community-favourited game-server metadata,
//! broadcast over the `GameServerAdded` control envelope. Wire shape
//! preserved from the pre-migration JSON form.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameServerInfo {
    /// 16-byte UUID hex (`gs_<32 hex>`).
    pub id: String,
    /// Game identifier (rich-presence id), serialized as string for JSON
    /// number-precision safety.
    pub game_id: String,
    pub label: String,
    /// `host:port` string the launcher uses.
    pub address: String,
    /// Hex-encoded pseudonym of the member who added the server.
    pub added_by: String,
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_server_info_roundtrip() {
        let g = GameServerInfo {
            id: "gs_01".into(),
            game_id: "halo".into(),
            label: "Friday CTF".into(),
            address: "10.0.0.1:27015".into(),
            added_by: "abcd".into(),
            created_at: 42,
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: GameServerInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(g, back);
    }
}

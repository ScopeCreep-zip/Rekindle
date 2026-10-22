//! The community expression list for the frontend (the community list and
//! details are the Host's, plan E3.1 D1b).

use crate::state::SharedState;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionInfoDto {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    /// Built in the backend from an allowlisted, byte-sniffed media type.
    pub inline_data_url: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
    /// Architecture §18.3 — present only on `kind == "soundboard"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound_meta: Option<rekindle_types::expression::SoundboardMeta>,
    /// Architecture §18.1 line 2455 — uploader's per-community pseudonym (hex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator_pseudonym: Option<String>,
    /// Architecture §18.1 line 2456 — wall-clock seconds at upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Architecture §18.1 line 2459 — gates `USE_EXTERNAL_EMOJIS`.
    pub available_to_peers: bool,
}

pub fn list_expressions_inner(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<ExpressionInfoDto>, String> {
    let expressions = crate::services::community::list_expressions(state, community_id)?;
    Ok(expressions
        .into_iter()
        .map(|expression| ExpressionInfoDto {
            expression_id: expression.expression_id,
            name: expression.name,
            kind: expression.kind,
            content_hash: expression.content_hash,
            inline_data_url: expression.inline_data_url,
            media_type: expression.media_type,
            animated: expression.animated,
            tags: expression.tags,
            sound_meta: expression.sound_meta,
            creator_pseudonym: expression.creator_pseudonym,
            created_at: expression.created_at,
            available_to_peers: expression.available_to_peers,
        })
        .collect())
}

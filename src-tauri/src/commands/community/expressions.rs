use tauri::State;

use crate::state::SharedState;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpressionInfoDto {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
}

#[tauri::command]
pub async fn upload_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    name: String,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, String> {
    crate::services::community::upload_emoji(state.inner(), &community_id, &name, bytes, animated)
        .await
}

#[tauri::command]
pub async fn delete_emoji(
    state: State<'_, SharedState>,
    community_id: String,
    expression_id: String,
) -> Result<(), String> {
    crate::services::community::delete_expression(state.inner(), &community_id, &expression_id)
        .await
}

#[tauri::command]
pub async fn list_expressions(
    state: State<'_, SharedState>,
    community_id: String,
) -> Result<Vec<ExpressionInfoDto>, String> {
    let expressions = crate::services::community::list_expressions(state.inner(), &community_id)?;
    Ok(expressions
        .into_iter()
        .map(|expression| ExpressionInfoDto {
            expression_id: expression.expression_id,
            name: expression.name,
            kind: expression.kind,
            content_hash: expression.content_hash,
            inline_data_base64: expression.inline_data_base64,
            media_type: expression.media_type,
            animated: expression.animated,
            tags: expression.tags,
        })
        .collect())
}

use base64::Engine as _;
use rand::RngCore;
use rekindle_governance::state::ExpressionState;
use rekindle_types::governance::GovernanceEntry;

use crate::state::SharedState;
use crate::state_helpers;

use super::governance;

const MAX_STATIC_EMOJI_BYTES: usize = 256 * 1024;
const MAX_ANIMATED_EMOJI_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub struct ExpressionInfo {
    pub expression_id: String,
    pub name: String,
    pub kind: String,
    pub content_hash: String,
    pub inline_data_base64: Option<String>,
    pub media_type: Option<String>,
    pub animated: bool,
    pub tags: Vec<String>,
}

pub async fn upload_emoji(
    state: &SharedState,
    community_id: &str,
    name: &str,
    bytes: Vec<u8>,
    animated: bool,
) -> Result<String, String> {
    validate_emoji_name(name)?;
    validate_emoji_bytes(&bytes, animated)?;

    let expression_id = random_16_bytes();
    let content_hash = blake3::hash(&bytes).to_hex().to_string();

    governance::write_entry(
        state,
        community_id,
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name: name.to_string(),
            kind: "emoji".to_string(),
            content_hash,
            inline_data: Some(bytes),
            animated,
            tags: Vec::new(),
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await?;

    Ok(hex::encode(expression_id))
}

pub async fn delete_expression(
    state: &SharedState,
    community_id: &str,
    expression_id_hex: &str,
) -> Result<(), String> {
    let expression_id: [u8; 16] = hex::decode(expression_id_hex)
        .map_err(|e| format!("invalid expression id: {e}"))?
        .try_into()
        .map_err(|_| "expression id must be 16 bytes")?;

    governance::write_entry(
        state,
        community_id,
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport: next_lamport(state, community_id)?,
        },
    )
    .await
}

pub fn list_expressions(
    state: &SharedState,
    community_id: &str,
) -> Result<Vec<ExpressionInfo>, String> {
    let mut expressions: Vec<_> = state_helpers::governance_state(state, community_id)
        .ok_or("governance state not loaded for this community")?
        .expressions
        .into_iter()
        .map(|(expression_id, expression)| to_expression_info(expression_id, expression))
        .collect();

    expressions.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.expression_id.cmp(&right.expression_id))
    });

    Ok(expressions)
}

fn next_lamport(state: &SharedState, community_id: &str) -> Result<u64, String> {
    if state_helpers::governance_state(state, community_id).is_none() {
        return Err("governance state not loaded for this community".into());
    }
    Ok(state_helpers::increment_lamport(state, community_id))
}

fn to_expression_info(expression_id: [u8; 16], expression: ExpressionState) -> ExpressionInfo {
    let media_type = expression
        .inline_data
        .as_ref()
        .and_then(|bytes| detect_media_type(bytes, expression.animated))
        .map(str::to_string);
    let inline_data_base64 = expression
        .inline_data
        .as_ref()
        .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));

    ExpressionInfo {
        expression_id: hex::encode(expression_id),
        name: expression.name,
        kind: expression.kind,
        content_hash: expression.content_hash,
        inline_data_base64,
        media_type,
        animated: expression.animated,
        tags: expression.tags,
    }
}

fn validate_emoji_name(name: &str) -> Result<(), String> {
    if !(2..=32).contains(&name.len()) {
        return Err("emoji name must be 2-32 characters".into());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err("emoji name may only contain letters, numbers, and underscores".into());
    }
    Ok(())
}

fn validate_emoji_bytes(bytes: &[u8], animated: bool) -> Result<(), String> {
    let max_len = if animated {
        MAX_ANIMATED_EMOJI_BYTES
    } else {
        MAX_STATIC_EMOJI_BYTES
    };
    if bytes.is_empty() {
        return Err("emoji upload cannot be empty".into());
    }
    if bytes.len() > max_len {
        return Err(format!("emoji upload exceeds {}KB limit", max_len / 1024));
    }

    match detect_media_type(bytes, animated) {
        Some(_) => Ok(()),
        None if animated => Err("animated emoji must be PNG, WebP, or GIF".into()),
        None => Err("emoji must be PNG or WebP".into()),
    }
}

fn detect_media_type(bytes: &[u8], animated: bool) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if animated && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }
    None
}

fn random_16_bytes() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
}

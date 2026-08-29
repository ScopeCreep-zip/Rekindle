//! Expression operations — play, delete, list.

use base64::Engine as _;
use rekindle_governance::state::ExpressionState;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::governance::GovernanceEntry;

use super::limits::detect_image_media_type;
use super::upload::next_lamport;
use crate::deps::{ChannelMessagingDeps, ExpressionView};
use crate::error::ChannelError;

pub fn play_soundboard<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    expression_id_hex: &str,
) -> Result<(), ChannelError> {
    let expr = list_expressions(deps, community_id)?
        .into_iter()
        .find(|e| e.expression_id.eq_ignore_ascii_case(expression_id_hex))
        .ok_or_else(|| ChannelError::InvalidId("expression not found".into()))?;
    if expr.kind != "soundboard" {
        return Err(ChannelError::InvalidId(
            "expression is not a soundboard sound".into(),
        ));
    }
    let actor_pseudonym = deps
        .my_pseudonym_hex(community_id)
        .ok_or_else(|| ChannelError::PseudonymKeyMissing(community_id.into()))?;
    let envelope = CommunityEnvelope::Control(ControlPayload::SoundboardPlay {
        channel_id: channel_id.to_string(),
        expression_id: expression_id_hex.to_string(),
        actor_pseudonym,
    });
    deps.send_to_mesh(community_id, &envelope)
}

/// Phase 19.f — delete a custom expression.
pub async fn delete_expression<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    expression_id_hex: &str,
) -> Result<(), ChannelError> {
    let expression_id: [u8; 16] = hex::decode(expression_id_hex)
        .map_err(|e| ChannelError::InvalidId(format!("invalid expression id: {e}")))?
        .try_into()
        .map_err(|_| ChannelError::InvalidId("expression id must be 16 bytes".into()))?;

    let lamport = next_lamport(deps, community_id)?;
    deps.write_governance_entry(
        community_id,
        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport,
        },
    )
    .await
}

/// Phase 19.f — list all expressions in a community, sorted by name.
pub fn list_expressions<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
) -> Result<Vec<ExpressionView>, ChannelError> {
    let gov = deps.governance_state(community_id).ok_or_else(|| {
        ChannelError::Adapter("governance state not loaded for this community".into())
    })?;
    let mut expressions: Vec<_> = gov
        .expressions
        .into_iter()
        .map(|(expression_id, expression)| {
            to_expression_view(deps, community_id, expression_id, expression)
        })
        .collect();
    expressions.sort_by(|l, r| {
        l.name
            .cmp(&r.name)
            .then_with(|| l.expression_id.cmp(&r.expression_id))
    });
    Ok(expressions)
}

fn to_expression_view<D: ChannelMessagingDeps>(
    deps: &D,
    community_id: &str,
    expression_id: [u8; 16],
    expression: ExpressionState,
) -> ExpressionView {
    let bytes = expression
        .attachment
        .as_ref()
        .and_then(|offer| deps.read_expression_bytes(community_id, offer));
    let media_type = bytes
        .as_deref()
        .and_then(|b| detect_image_media_type(b, expression.animated))
        .map(str::to_string);
    let inline_data_base64 = bytes
        .as_deref()
        .map(|b| base64::engine::general_purpose::STANDARD.encode(b));

    ExpressionView {
        expression_id: hex::encode(expression_id),
        name: expression.name,
        kind: expression.kind,
        content_hash: expression.content_hash,
        inline_data_base64,
        media_type,
        animated: expression.animated,
        tags: expression.tags,
        sound_meta: expression.sound_meta,
        creator_pseudonym: expression.creator_pseudonym.map(|p| hex::encode(p.0)),
        created_at: expression.created_at,
        available_to_peers: expression.available_to_peers,
    }
}

//! Architecture §28.8 — link preview wire type.
//!
//! Link previews are fetched out-of-band by the *sender's* device
//! (direct HTTP, not Veilid) and broadcast to peers via the gossip
//! `ControlPayload::LinkPreview` variant. Receivers display them
//! inline; trust gating is enforced reader-side via the
//! `EMBED_LINKS` permission.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub message_id: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    pub fetched_at: u64,
}

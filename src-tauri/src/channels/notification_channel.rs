use serde::Serialize;

/// System-level notification events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", content = "data")]
pub enum NotificationEvent {
    SystemAlert { title: String, body: String },
    UpdateAvailable { version: String },
}

/// Pushed to the frontend whenever network-relevant state changes
/// (attachment, readiness, or route allocation) so the `NetworkIndicator`
/// can update instantly instead of polling.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkStatusEvent {
    /// Raw Veilid `AttachmentState` string (e.g. "detached", "attaching", "`attached_good`").
    pub attachment_state: String,
    pub is_attached: bool,
    pub public_internet_ready: bool,
    /// Whether we have an allocated private route for receiving messages.
    pub has_route: bool,
}

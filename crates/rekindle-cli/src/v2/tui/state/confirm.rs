//! Confirmation dialog state.

/// Action awaiting user confirmation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum PendingConfirmAction {
    LeaveCommunity { community: String },
    RemoveFriend { peer_key: String },
    KickMember { community: String, pseudonym: String },
    BanMember { community: String, pseudonym: String },
    TimeoutMember { community: String, pseudonym: String, duration_secs: u64 },
    ApproveMember { community: String, pseudonym: String },
    RejectMember { community: String, pseudonym: String },
    RevokeInvite { community: String, invite_code: String },
    UnpinMessage { community: String, channel: String, message_id: String },
    LeaveVoice,
}

/// Two-button confirmation dialog.
#[derive(Debug)]
pub struct ConfirmState {
    pub prompt: String,
    pub consequence: String,
    /// `true` = confirm button focused, `false` = cancel focused.
    pub confirm_focused: bool,
    pub visible: bool,
    pub action: Option<PendingConfirmAction>,
}

impl ConfirmState {
    pub fn new() -> Self {
        Self {
            prompt: String::new(),
            consequence: String::new(),
            confirm_focused: false,
            visible: false,
            action: None,
        }
    }

    pub fn show(
        &mut self,
        prompt: impl Into<String>,
        consequence: impl Into<String>,
        action: PendingConfirmAction,
    ) {
        self.prompt = prompt.into();
        self.consequence = consequence.into();
        self.confirm_focused = false;
        self.visible = true;
        self.action = Some(action);
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.action = None;
    }

    pub fn toggle_focus(&mut self) {
        self.confirm_focused = !self.confirm_focused;
    }

    /// Takes the pending action if confirm is focused.
    #[must_use]
    pub fn take_confirmed(&mut self) -> Option<PendingConfirmAction> {
        if self.confirm_focused {
            self.visible = false;
            self.action.take()
        } else {
            self.hide();
            None
        }
    }
}

impl Default for ConfirmState {
    fn default() -> Self {
        Self::new()
    }
}

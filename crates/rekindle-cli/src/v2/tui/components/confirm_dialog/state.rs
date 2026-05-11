//! Confirmation dialog state.

pub struct ConfirmDialogState {
    pub prompt: String,
    pub consequence: String,
    pub confirm_focused: bool,
    pub visible: bool,
}

impl ConfirmDialogState {
    pub fn new() -> Self {
        Self { prompt: String::new(), consequence: String::new(), confirm_focused: false, visible: false }
    }

    pub fn show(&mut self, prompt: impl Into<String>, consequence: impl Into<String>) {
        self.prompt = prompt.into();
        self.consequence = consequence.into();
        self.confirm_focused = false;
        self.visible = true;
    }

    pub fn hide(&mut self) { self.visible = false; }
    pub fn toggle_focus(&mut self) { self.confirm_focused = !self.confirm_focused; }
    pub fn is_confirmed(&self) -> bool { self.confirm_focused }
}

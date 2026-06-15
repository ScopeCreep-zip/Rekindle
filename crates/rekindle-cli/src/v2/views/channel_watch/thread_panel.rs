//! Thread panel — split pane showing thread messages alongside the main channel.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Frame;

use crate::v2::tui::components::Component;
use crate::v2::tui::components::input_box::InputBox;
use crate::v2::tui::components::message_list::MessageList;

pub struct ThreadPanel {
    pub visible: bool,
    pub thread_id: String,
    pub thread_name: String,
    pub message_list: MessageList,
    pub input_box: InputBox,
}

impl ThreadPanel {
    pub fn new() -> Self {
        Self {
            visible: false,
            thread_id: String::new(),
            thread_name: String::new(),
            message_list: MessageList::new(String::new(), String::new()),
            input_box: InputBox::new(),
        }
    }

    pub fn open(&mut self, thread_id: String, thread_name: String) {
        self.visible = true;
        self.thread_id = thread_id.clone();
        self.thread_name = thread_name;
        self.message_list = MessageList::new(String::new(), thread_id);
        self.input_box = InputBox::new();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.thread_id.clear();
        self.thread_name.clear();
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        if !self.visible { return; }

        let [msg_area, input_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(3),
        ]).areas(area);

        self.message_list.set_focused(focused);
        self.message_list.draw_messages(frame, msg_area);

        self.input_box.set_focused(focused);
        self.input_box.draw(frame, input_area);
    }
}

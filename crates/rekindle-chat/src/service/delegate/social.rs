//! Social delegation — reactions, pins, events, threads, game servers.

use crate::ChatError;
use super::super::ChatService;

impl ChatService {
    pub async fn add_reaction(&self, gov: &str, ch: &str, msg: &str, emoji: &str) -> Result<(), ChatError> {
        self.community.add_reaction(gov, ch, msg, emoji).await
    }

    pub async fn remove_reaction(&self, gov: &str, ch: &str, msg: &str, emoji: &str) -> Result<(), ChatError> {
        self.community.remove_reaction(gov, ch, msg, emoji).await
    }

    pub async fn pin_message(&self, gov: &str, ch: &str, msg: &str) -> Result<(), ChatError> {
        self.community.pin_message(gov, ch, msg).await
    }

    pub async fn unpin_message(&self, gov: &str, ch: &str, msg: &str) -> Result<(), ChatError> {
        self.community.unpin_message(gov, ch, msg).await
    }

    pub async fn create_event(
        &self, gov: &str, title: &str, description: &str, start_time: u64,
        end_time: Option<u64>, channel_id: Option<&str>, max_attendees: Option<u32>,
    ) -> Result<String, ChatError> {
        self.community.create_event(gov, title, description, start_time, end_time, channel_id, max_attendees).await
    }

    pub async fn update_event(
        &self, gov: &str, event_id: &str, title: &str, description: &str,
        start_time: u64, end_time: Option<u64>, max_attendees: Option<u32>,
    ) -> Result<(), ChatError> {
        self.community.update_event(gov, event_id, title, description, start_time, end_time, max_attendees).await
    }

    pub async fn delete_event(&self, gov: &str, event_id: &str) -> Result<(), ChatError> {
        self.community.delete_event(gov, event_id).await
    }

    pub async fn rsvp_event(&self, gov: &str, event_id: &str, status: &str) -> Result<(), ChatError> {
        self.community.rsvp_event(gov, event_id, status).await
    }

    pub async fn event_reminder(&self, gov: &str, event_id: &str, title: &str, minutes: u32) -> Result<(), ChatError> {
        self.community.event_reminder(gov, event_id, title, minutes).await
    }

    pub async fn create_thread(
        &self, gov: &str, ch: &str, parent_msg: &str, title: &str, auto_archive: u32,
    ) -> Result<String, ChatError> {
        self.community.create_thread(gov, ch, parent_msg, title, auto_archive).await
    }

    pub async fn thread_message(
        &self, gov: &str, thread_id: &str, ciphertext: Vec<u8>, mek_gen: u64, reply_to: Option<&str>,
    ) -> Result<String, ChatError> {
        self.community.thread_message(gov, thread_id, ciphertext, mek_gen, reply_to).await
    }

    pub async fn archive_thread(&self, gov: &str, thread_id: &str, archived: bool) -> Result<(), ChatError> {
        self.community.archive_thread(gov, thread_id, archived).await
    }

    pub async fn add_game_server(&self, gov: &str, game_id: &str, label: &str, address: &str) -> Result<String, ChatError> {
        self.community.add_game_server(gov, game_id, label, address).await
    }

    pub async fn remove_game_server(&self, gov: &str, server_id: &str) -> Result<(), ChatError> {
        self.community.remove_game_server(gov, server_id).await
    }
}

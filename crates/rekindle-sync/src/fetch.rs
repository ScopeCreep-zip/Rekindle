//! Fetch queue with bounded retry metadata.

use std::collections::VecDeque;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchTask {
    pub record_key: String,
    pub subkey: u32,
    pub expected_hash: Option<String>,
    pub attempt: u32,
}

#[derive(Debug, Clone, Default)]
pub struct FetchQueue {
    queue: VecDeque<FetchTask>,
}

impl FetchQueue {
    pub fn push(&mut self, task: FetchTask) {
        self.queue.push_back(task);
    }

    pub fn pop(&mut self) -> Option<FetchTask> {
        self.queue.pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

pub fn retry_backoff(attempt: u32) -> Duration {
    Duration::from_secs(1 << attempt.min(2))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{retry_backoff, FetchQueue, FetchTask};

    #[test]
    fn queue_is_fifo() {
        let mut queue = FetchQueue::default();
        queue.push(FetchTask {
            record_key: "a".into(),
            subkey: 1,
            expected_hash: None,
            attempt: 0,
        });
        queue.push(FetchTask {
            record_key: "b".into(),
            subkey: 2,
            expected_hash: Some("hash".into()),
            attempt: 1,
        });

        assert_eq!(queue.pop().unwrap().record_key, "a");
        assert_eq!(queue.pop().unwrap().record_key, "b");
        assert!(queue.is_empty());
    }

    #[test]
    fn retry_backoff_caps_at_four_seconds() {
        assert_eq!(retry_backoff(0), Duration::from_secs(1));
        assert_eq!(retry_backoff(1), Duration::from_secs(2));
        assert_eq!(retry_backoff(2), Duration::from_secs(4));
        assert_eq!(retry_backoff(5), Duration::from_secs(4));
    }

    /// Three-path independence: gossip notification (Path 2) can populate the
    /// fetch queue independently — no need for SMPL write confirmation or
    /// inspect polling. The notification carries subkey_index and content_hash,
    /// which is all FetchQueue needs to schedule a DHT fetch.
    #[test]
    fn gossip_notification_creates_fetch_task_independently() {
        let mut queue = FetchQueue::default();

        // Simulate receiving a gossip MessageNotification with metadata only
        queue.push(FetchTask {
            record_key: "channel_rec_abc".into(),
            subkey: 7, // sender's SMPL subkey_index from notification
            expected_hash: Some(
                "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4".into(),
            ), // blake3 content_hash from notification
            attempt: 0,
        });

        assert_eq!(queue.len(), 1);

        let task = queue.pop().unwrap();
        // The fetch task has everything needed to retrieve and verify from DHT
        assert_eq!(task.subkey, 7);
        assert!(task.expected_hash.is_some());
        assert_eq!(task.attempt, 0);
    }
}

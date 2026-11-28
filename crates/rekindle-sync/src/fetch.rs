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
#[path = "fetch_tests.rs"]
mod tests;

//! Record warming cadence.

use std::time::{Duration, Instant};

pub const RECORD_WARM_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct RecordWarmer {
    last_cycle: Instant,
}

impl RecordWarmer {
    pub fn new(now: Instant) -> Self {
        Self { last_cycle: now }
    }

    pub fn due_records<'a>(&self, now: Instant, records: &'a [String]) -> &'a [String] {
        if now.saturating_duration_since(self.last_cycle) >= RECORD_WARM_INTERVAL {
            records
        } else {
            &[]
        }
    }

    pub fn mark_warmed(&mut self, now: Instant) {
        self.last_cycle = now;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::RecordWarmer;

    #[test]
    fn warming_cycles_every_five_minutes() {
        let start = Instant::now();
        let records = vec!["gov".to_string(), "chan".to_string()];
        let warmer = RecordWarmer::new(start);
        assert!(warmer
            .due_records(start + Duration::from_secs(299), &records)
            .is_empty());
        assert_eq!(
            warmer.due_records(start + Duration::from_secs(300), &records),
            records.as_slice()
        );
    }
}

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

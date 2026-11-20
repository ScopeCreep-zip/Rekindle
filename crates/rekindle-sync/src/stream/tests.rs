use parking_lot::Mutex;
use rekindle_records::chain::ChainEntry;

use super::{classify, Held, Rule, SinkError, StreamEngine, StreamSink};

#[test]
fn the_receive_rule_applies_the_next_passes_older_and_holds_a_gap() {
    assert_eq!(classify(0, 1), Rule::Apply);
    assert_eq!(classify(4, 5), Rule::Apply);
    assert_eq!(classify(4, 4), Rule::Duplicate);
    assert_eq!(classify(4, 2), Rule::Duplicate);
    assert_eq!(classify(4, 7), Rule::Hold);
    assert_eq!(classify(u64::MAX, 1), Rule::Duplicate);
}

/// One diff per stream: a trigger during it makes it read once more, and
/// another stream runs alongside.
#[test]
fn a_trigger_during_a_diff_coalesces_into_one_more_pass() {
    let engine = StreamEngine::default();
    let a = ("dm".to_string(), "alice".to_string());
    let b = ("dm".to_string(), "bob".to_string());
    assert!(engine.claim(&a));
    assert!(engine.claim(&b));
    assert!(!engine.claim(&a));
    assert!(!engine.claim(&a));
    assert!(!engine.release(&a, false), "read again");
    assert!(engine.release(&a, false), "drained");
    assert!(engine.claim(&a));
    assert!(!engine.claim(&a));
    assert!(engine.release(&a, true), "a failed pass ends the diff");
    assert!(engine.release(&b, false));
}

/// A sink that holds only the entries whose content has arrived, and
/// commits the cursor after each one it holds.
#[derive(Default)]
struct Sink {
    arrived: Mutex<Vec<u64>>,
    cursor: Mutex<u64>,
    caught_up: Mutex<usize>,
}

#[async_trait::async_trait]
impl StreamSink for Sink {
    async fn cursor(&self, _: &str, _: &str) -> Result<u64, SinkError> {
        Ok(*self.cursor.lock())
    }
    async fn apply(&self, _: &str, _: &str, entry: &ChainEntry) -> Result<Held, SinkError> {
        if !self.arrived.lock().contains(&entry.seq) {
            return Ok(Held::Pending);
        }
        *self.cursor.lock() = entry.seq;
        Ok(Held::Stored)
    }
    async fn history_gap(&self, _: &str, _: &str, through: u64) -> Result<(), SinkError> {
        *self.cursor.lock() = through;
        Ok(())
    }
    async fn caught_up(&self, _: &str, _: &str) {
        *self.caught_up.lock() += 1;
    }
}

fn entries(seqs: std::ops::RangeInclusive<u64>) -> Vec<ChainEntry> {
    seqs.map(|seq| ChainEntry {
        seq,
        ciphertext: vec![u8::try_from(seq).unwrap()],
    })
    .collect()
}

/// Live finding L-ACK1: the head reached entry 2 before entry 1's content
/// was held. The cursor (a `DmAck`'s `through`) stays below entry 1, entry
/// 2 is not applied past it, and only once entry 1 is stored does the
/// cursor cover both: it is the contiguous prefix of entries held.
#[tokio::test(flavor = "current_thread")]
async fn a_head_past_an_entry_not_held_never_moves_the_cursor_over_it() {
    let sink = Sink::default();
    sink.arrived.lock().push(2);
    let read = entries(1..=2);
    let done = StreamEngine::apply_entries(&sink, ("dm", "bob"), 0, &read)
        .await
        .unwrap();
    assert_eq!(done.pending, Some(1));
    assert_eq!(done.applied, 0);
    assert_eq!(*sink.cursor.lock(), 0, "no ack covers entry 1 or 2 yet");

    sink.arrived.lock().push(1);
    let done = StreamEngine::apply_entries(&sink, ("dm", "bob"), 0, &read)
        .await
        .unwrap();
    assert_eq!((done.pending, done.applied), (None, 2));
    assert_eq!(*sink.cursor.lock(), 2, "both held: the cursor covers both");
}

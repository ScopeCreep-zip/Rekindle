use parking_lot::Mutex;
use rekindle_records::chain::ChainEntry;

use super::{after_mark, classify, Held, Rule, SinkError, StreamEngine, StreamError, StreamSink};

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
    async fn history_gap(
        &self,
        _: &str,
        _: &str,
        through: u64,
        _: Option<&ChainEntry>,
    ) -> Result<(), SinkError> {
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

/// One pass over `read` from the sink's cursor, as `diff_chain` runs it
/// (the chain read itself is the record crate's, tested there).
async fn pass_over(sink: &Sink, read: &[ChainEntry]) -> Result<super::Diffed, StreamError> {
    let cursor = *sink.cursor.lock();
    StreamEngine::apply_entries(sink, ("channel_outbox:c1", "author"), cursor, read).await
}

/// Finding CB1: a joiner's catch-up reads an author's outbox before its
/// first community key lands, so the entry waits ([`Held::Pending`]); the
/// key's install starts another catch-up while that read still runs, and
/// the claim it makes coalesces into it. The diff must read again and
/// hold the entry then, not drop the install's trigger until the author's
/// next post. The outbox sink once returned an error for the wait, and a
/// failed pass ends the diff with the claim dropped
/// ([`a_failed_pass_drops_a_trigger_that_landed_during_it`]).
#[tokio::test(flavor = "current_thread")]
async fn a_trigger_during_a_pass_held_pending_reads_again_and_holds_the_entry() {
    let engine = StreamEngine::default();
    let sink = Sink::default();
    let key = ("channel_outbox:c1".to_string(), "author".to_string());
    let read = entries(1..=1);
    let passes = Mutex::new(0_usize);
    let total = engine
        .run_passes(&sink, ("channel_outbox:c1", "author"), || async {
            let n = {
                let mut passes = passes.lock();
                *passes += 1;
                *passes
            };
            let done = pass_over(&sink, &read).await;
            if n == 1 {
                // The key's install lands during the read: its catch-up
                // coalesces, and the entry's key is held from now on.
                assert!(!engine.claim(&key), "the install's catch-up coalesces");
                sink.arrived.lock().push(1);
            }
            done
        })
        .await
        .expect("a wait is not a failure");
    assert_eq!(*passes.lock(), 2, "the coalesced trigger reads again");
    assert_eq!((total.applied, total.pending), (1, None));
    assert_eq!(*sink.cursor.lock(), 1, "the entry is held");
    assert_eq!(*sink.caught_up.lock(), 1);
    assert!(engine.claim(&key), "the diff released the stream");
}

/// Why a wait must not be an error: a failed pass ends the diff, and a
/// trigger that coalesced into it is gone with it.
#[tokio::test(flavor = "current_thread")]
async fn a_failed_pass_drops_a_trigger_that_landed_during_it() {
    let engine = StreamEngine::default();
    let sink = Sink::default();
    let key = ("channel_outbox:c1".to_string(), "author".to_string());
    let passes = Mutex::new(0_usize);
    let failed = engine
        .run_passes(&sink, ("channel_outbox:c1", "author"), || async {
            *passes.lock() += 1;
            assert!(!engine.claim(&key));
            Err::<super::Diffed, _>(StreamError::Sink(SinkError("store".into())))
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(*passes.lock(), 1, "no second pass after a failure");
    assert_eq!(*sink.caught_up.lock(), 0);
    assert!(
        engine.claim(&key),
        "the stream is released, the trigger dropped"
    );
}

/// G4: the sink is handed the first entry after the retire mark, the one at
/// `through + 1`; a read that holds none, or starts past it, hands none.
#[test]
fn a_gap_is_handed_the_entry_right_after_the_mark() {
    let read = entries(5..=7);
    assert_eq!(after_mark(&read, 4).map(|e| e.seq), Some(5));
    assert_eq!(after_mark(&read, 3), None, "6 is not right after 3");
    assert_eq!(after_mark(&[], 4), None, "nothing after the mark");
    assert_eq!(after_mark(&read, u64::MAX), None);
}

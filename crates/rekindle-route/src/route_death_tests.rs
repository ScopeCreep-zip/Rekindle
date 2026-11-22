use std::fmt::Write as _;
use std::sync::Arc;

use parking_lot::Mutex;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Metadata, Subscriber};

use super::*;

/// Records every "own route died" event's fields as `name=value` text.
#[derive(Clone, Default)]
pub(crate) struct Deaths(Arc<Mutex<Vec<String>>>);

struct Fields(String, bool);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.1 = format!("{value:?}") == "own route died";
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

impl Subscriber for Deaths {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut fields = Fields(String::new(), false);
        event.record(&mut fields);
        if fields.1 {
            self.0.lock().push(fields.0);
        }
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

impl Deaths {
    /// The death events `f` emits on this thread.
    pub(crate) fn during(f: impl FnOnce()) -> Vec<String> {
        let deaths = Self::default();
        tracing::subscriber::with_default(deaths.clone(), f);
        let got = deaths.0.lock().clone();
        got
    }
}

/// The longest run of hex digits in `text`: an id, a key or a hash shows
/// as one of 16 or more.
pub(crate) fn longest_hex_run(text: &str) -> usize {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .map(str::len)
        .max()
        .unwrap_or(0)
}

/// Asserts `line` carries no identifier-shaped field.
pub(crate) fn assert_identifier_free(line: &str) {
    assert!(
        longest_hex_run(line) < 16,
        "identifier-shaped run in {line:?}"
    );
}

#[test]
fn the_hex_check_catches_an_id() {
    assert!(longest_hex_run("route=1f2e3d4c5b6a79881f2e3d4c age_s=3") >= 16);
    assert!(longest_hex_run("class=General age_s=1234567") < 16);
}

#[test]
fn a_death_logs_counts_and_durations_only() {
    let born = Instant::now();
    let death = RouteDeath::new(
        RouteClass::General,
        DeathCause::Rotated,
        Some(3),
        born,
        true,
        born + Duration::from_secs(140),
    );
    assert_eq!(
        death.fields(),
        "class=General cause=rotated hops=3 age_s=140 node_ready=true"
    );
    let lines = Deaths::during(|| death.log());
    assert_eq!(lines.len(), 1);
    assert_identifier_free(&lines[0]);
    assert!(lines[0].contains("age_s=140"), "{}", lines[0]);
}

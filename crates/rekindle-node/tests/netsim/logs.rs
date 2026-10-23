//! A per-run log capture (plan E2.B S10): each scenario's thread gets its
//! own subscriber (`tracing::subscriber::set_default`, no global one, so
//! parallel tests never share counts), which prints as `RUST_LOG` asks and
//! counts every WARN and ERROR event by the node whose [`NODE_SPAN`] it ran
//! in. The harness opens that span around a node's Host start and its
//! requests; a task the Host spawns runs in its spawner's span
//! (`SessionScope` and the transport's loops instrument it), so its events
//! count under that node too. Only an event outside every node span (a
//! bare `tokio::spawn` in a test) counts under `None`. Events on other
//! threads (a database thread) are not counted.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use parking_lot::Mutex;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer};

/// The span naming the node a Host's work runs for (field `node`).
pub const NODE_SPAN: &str = "netsim_node";

/// WARN and ERROR events counted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub warn: usize,
    pub error: usize,
}

type Tally = Arc<Mutex<BTreeMap<Option<usize>, Counts>>>;
/// Each WARN and ERROR event's text (its message and fields), by node.
type Lines = Arc<Mutex<Vec<(Option<usize>, String)>>>;

thread_local! {
    static RUN: RefCell<Option<(tracing::subscriber::DefaultGuard, Tally, Lines)>> =
        const { RefCell::new(None) };
}

/// Install this thread's capture (once per run's thread).
pub fn install() {
    RUN.with(|run| {
        let mut run = run.borrow_mut();
        if run.is_none() {
            let tally = Tally::default();
            let lines = Lines::default();
            let subscriber = tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_test_writer()
                        .with_filter(EnvFilter::from_default_env()),
                )
                .with(Capture(Arc::clone(&tally), Arc::clone(&lines)));
            *run = Some((tracing::subscriber::set_default(subscriber), tally, lines));
        }
    });
}

/// The counts so far, by node (`None`: no node span in scope).
pub fn counts() -> BTreeMap<Option<usize>, Counts> {
    RUN.with(|run| {
        run.borrow()
            .as_ref()
            .map(|(_, t, _)| t.lock().clone())
            .unwrap_or_default()
    })
}

/// The WARN and ERROR events so far that contain `needle`, any node.
pub fn warned(needle: &str) -> Vec<String> {
    RUN.with(|run| {
        run.borrow()
            .as_ref()
            .map(|(_, _, lines)| {
                lines
                    .lock()
                    .iter()
                    .filter(|(_, text)| text.contains(needle))
                    .map(|(node, text)| format!("node {node:?}: {text}"))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Every node's counts summed.
pub fn total() -> Counts {
    counts().values().fold(Counts::default(), |a, c| Counts {
        warn: a.warn + c.warn,
        error: a.error + c.error,
    })
}

struct NodeIndex(usize);

struct NodeField(Option<usize>);

impl Visit for NodeField {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "node" {
            self.0 = usize::try_from(value).ok();
        }
    }

    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

/// An event's message and fields as one line.
#[derive(Default)]
struct Text(String);

impl Visit for Text {
    fn record_str(&mut self, field: &Field, value: &str) {
        let _ = write!(self.0, "{}={value} ", field.name());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let _ = write!(self.0, "{}={value:?} ", field.name());
    }
}

struct Capture(Tally, Lines);

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for Capture {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if attrs.metadata().name() != NODE_SPAN {
            return;
        }
        let mut field = NodeField(None);
        attrs.record(&mut field);
        if let (Some(node), Some(span)) = (field.0, ctx.span(id)) {
            span.extensions_mut().insert(NodeIndex(node));
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::ERROR {
            return;
        }
        let node = ctx.event_scope(event).and_then(|scope| {
            scope
                .from_root()
                .filter_map(|s| s.extensions().get::<NodeIndex>().map(|n| n.0))
                .last()
        });
        let mut text = Text::default();
        event.record(&mut text);
        self.1.lock().push((node, text.0));
        let mut tally = self.0.lock();
        let counts = tally.entry(node).or_default();
        if level == Level::ERROR {
            counts.error += 1;
        } else {
            counts.warn += 1;
        }
    }
}

/// The capture counts an event by the node span it ran in, one from a
/// `SessionScope` task spawned in that span under the same node, and one
/// from a bare spawn (no span) under `None`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_capture_counts_by_node() {
    use tracing::Instrument;
    install();
    let base = counts();
    async { tracing::warn!("in node 3") }
        .instrument(tracing::info_span!("netsim_node", node = 3_u64))
        .await;
    let scope = rekindle_lifecycle::SessionScope::new("test", Arc::new(|_| {}));
    let (tx, rx) = tokio::sync::oneshot::channel();
    tracing::info_span!("netsim_node", node = 3_u64).in_scope(|| {
        scope
            .spawn("warns", async move {
                tracing::warn!("scope task of node 3");
                let _ = tx.send(());
            })
            .expect("open scope");
    });
    rx.await.expect("task ran");
    tokio::spawn(async { tracing::error!("spawned") })
        .await
        .expect("task");
    let now = counts();
    let delta = |k: Option<usize>| {
        let (a, b) = (
            now.get(&k).copied().unwrap_or_default(),
            base.get(&k).copied().unwrap_or_default(),
        );
        (a.warn - b.warn, a.error - b.error)
    };
    assert_eq!(delta(Some(3)), (2, 0));
    assert_eq!(delta(None), (0, 1));
}

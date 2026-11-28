//! A per-run log capture (plan E2.B S10): each scenario's thread is armed
//! with its own tally (parallel tests never share counts), and the one
//! global subscriber prints as `RUST_LOG` asks and counts every WARN and
//! ERROR event on an armed thread by the node whose [`NODE_SPAN`] it ran
//! in, keeping the text of every INFO, WARN and ERROR event. The harness
//! opens that span around a node's Host start and its requests; a task the
//! Host spawns runs in its spawner's span (`SessionScope` and the
//! transport's loops instrument it), so its events count under that node
//! too. Only an event outside every node span (a bare `tokio::spawn` in a
//! test) counts under `None`. Events on other threads (a database thread)
//! are not counted.
//!
//! Not a thread-local `set_default` subscriber per run: tracing-core 0.1.36
//! caches a callsite's interest at its first hit (`DefaultCallsite::
//! register`, callsite.rs:308-321), and while one dispatcher is registered
//! (`has_just_one`, callsite.rs:551-557) asks only the hitting thread's
//! default (`Rebuilder::JustOne`, callsite.rs:544-549, 563-567). A thread
//! with none (a database thread, a run's thread before its install)
//! answers `Interest::never` (dispatcher.rs:379-397), and the line is then
//! skipped on every thread, a run's too: its WARN went uncounted and a "no
//! WARN" assertion passed falsely. So the global subscriber's capture
//! answers `Interest::sometimes` for every INFO-or-above callsite
//! (`DynFilterFn`, filter_fn.rs:577-622) and decides per event whether this
//! thread is armed, as `14e19578` did for the node lib's capture.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use parking_lot::Mutex;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::filter::{dynamic_filter_fn, FilterExt, LevelFilter};
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
/// Each INFO, WARN and ERROR event's level and text (its message and
/// fields), by node.
type Lines = Arc<Mutex<Vec<(Option<usize>, Level, String)>>>;

/// A run's capture: its counts and its lines.
#[derive(Clone, Default)]
struct Run(Tally, Lines);

thread_local! {
    /// The run this thread's events are captured into, once armed.
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
}

/// Whether this thread is armed.
fn armed() -> bool {
    RUN.with(|run| run.borrow().is_some())
}

/// Install the global subscriber (once per test binary) and arm this
/// thread (once per run's thread).
pub fn install() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_test_writer()
                    .with_filter(
                        EnvFilter::from_default_env().and(dynamic_filter_fn(|_, _| armed())),
                    ),
            )
            .with(
                Capture.with_filter(
                    dynamic_filter_fn(|meta: &Metadata<'_>, _| {
                        *meta.level() <= Level::INFO && (meta.is_span() || armed())
                    })
                    .with_max_level_hint(LevelFilter::INFO),
                ),
            );
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other subscriber in the netsim binary");
    });
    RUN.with(|run| {
        run.borrow_mut().get_or_insert_with(Run::default);
    });
}

/// The counts so far, by node (`None`: no node span in scope).
pub fn counts() -> BTreeMap<Option<usize>, Counts> {
    RUN.with(|run| {
        run.borrow()
            .as_ref()
            .map(|Run(tally, _)| tally.lock().clone())
            .unwrap_or_default()
    })
}

/// The WARN and ERROR events so far that contain `needle`, any node.
pub fn warned(needle: &str) -> Vec<String> {
    lines(|_, level, text| level <= Level::WARN && text.contains(needle))
}

/// The INFO, WARN and ERROR events node `node` logged so far that contain
/// every one of `needles`.
pub fn logged(node: usize, needles: &[&str]) -> Vec<String> {
    lines(|at, _, text| at == Some(node) && needles.iter().all(|n| text.contains(n)))
}

/// The captured events `keep` picks, each as `node …: text`.
fn lines(keep: impl Fn(Option<usize>, Level, &str) -> bool) -> Vec<String> {
    RUN.with(|run| {
        run.borrow()
            .as_ref()
            .map(|Run(_, lines)| {
                lines
                    .lock()
                    .iter()
                    .filter(|(node, level, text)| keep(*node, *level, text))
                    .map(|(node, _, text)| format!("node {node:?}: {text}"))
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

/// Counts and keeps the events of armed threads, into the armed run.
struct Capture;

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
        if level > Level::INFO {
            return;
        }
        let node = ctx.event_scope(event).and_then(|scope| {
            scope
                .from_root()
                .filter_map(|s| s.extensions().get::<NodeIndex>().map(|n| n.0))
                .last()
        });
        let Some(Run(tally, lines)) = RUN.with(|run| run.borrow().clone()) else {
            return;
        };
        let mut text = Text::default();
        event.record(&mut text);
        lines.lock().push((node, level, text.0));
        if level == Level::INFO {
            return;
        }
        let mut tally = tally.lock();
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

/// The probe's one callsite, hit from both threads below.
fn warn_probe() {
    tracing::warn!("netsim logs probe, first hit on another thread");
}

/// A WARN whose callsite is first hit on a thread that is not armed (a
/// database thread, say) while this run captures still counts here: the
/// interleaving that cached the callsite `never` under a per-thread
/// `set_default` (see the module docs), when this run's was the only
/// dispatcher. Fails with that capture when run alone (`cargo test
/// --test netsim logs::a_line_first_hit`), the one-dispatcher case.
#[test]
fn a_line_first_hit_on_another_thread_still_counts() {
    install();
    std::thread::spawn(warn_probe).join().expect("probe thread");
    warn_probe();
    assert_eq!(
        warned("netsim logs probe").len(),
        1,
        "the probe's WARN on this thread was captured"
    );
}

//! Requests being dispatched, one task each.
//!
//! Each request runs as its own task, so a slow handler never blocks the
//! subscriber loop, and a panicking handler costs only its own request:
//! the panic surfaces here as a `JoinError` and becomes a 500 for that
//! request's correlation id, while the loop keeps serving.
//!
//! Each task keeps its request's kind (the `IpcRequest` variant name, never
//! its fields: the threat model keeps identifiers and message text out of
//! logs) and when it started, so the shutdown drain can say which requests
//! it waited for and which it cut off (live R6-1).

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::IpcResponse;
use tokio::task::{Id, JoinSet};
use tokio::time::Instant;
use tracing::Instrument as _;
use uuid::Uuid;

/// Who a response goes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Correlation {
    pub id: Uuid,
    pub level: SecurityLevel,
}

/// A finished request.
#[derive(Debug)]
pub(crate) struct Finished {
    pub correlation: Correlation,
    pub response: IpcResponse,
    /// The handler panicked: the response is a 500 and shared state may be
    /// half-mutated.
    pub panicked: bool,
}

/// A request still running: its kind and how long it has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Running {
    /// The `IpcRequest` variant name ([`IpcRequest::name`]), without fields.
    ///
    /// [`IpcRequest::name`]: rekindle_ipc::protocol::IpcRequest::name
    pub kind: &'static str,
    pub ran: Duration,
}

/// What a shutdown drain did.
#[derive(Debug, Default)]
pub(crate) struct Drained {
    /// Every request in flight when the drain started.
    pub waited: Vec<Running>,
    /// The requests still running at the deadline, aborted; each still
    /// owes its requester an answer.
    pub aborted: Vec<(Correlation, Running)>,
}

/// Whom a task answers, what it is, and since when.
struct Owner {
    correlation: Correlation,
    kind: &'static str,
    started: Instant,
}

impl Owner {
    fn running(&self) -> Running {
        Running {
            kind: self.kind,
            ran: self.started.elapsed(),
        }
    }
}

/// The dispatch tasks in flight, keyed back to their requests.
#[derive(Default)]
pub(crate) struct InFlight {
    tasks: JoinSet<IpcResponse>,
    owners: HashMap<Id, Owner>,
}

impl InFlight {
    /// Run `dispatch` (a request of `kind`) as its own task, answering
    /// `correlation`, in the span it was started in (its log lines name the
    /// host they ran for).
    pub(crate) fn spawn<F>(&mut self, correlation: Correlation, kind: &'static str, dispatch: F)
    where
        F: Future<Output = IpcResponse> + Send + 'static,
    {
        let handle = self.tasks.spawn(dispatch.in_current_span());
        self.owners.insert(
            handle.id(),
            Owner {
                correlation,
                kind,
                started: Instant::now(),
            },
        );
    }

    /// The requests running now, longest-running first.
    pub(crate) fn running(&self) -> Vec<Running> {
        let mut running: Vec<Running> = self.owners.values().map(Owner::running).collect();
        running.sort_by(|a, b| b.ran.cmp(&a.ran).then(a.kind.cmp(b.kind)));
        running
    }

    /// The next finished request; `None` when nothing is in flight.
    /// Cancel-safe: a `select!` that drops this loses nothing.
    pub(crate) async fn next(&mut self) -> Option<Finished> {
        loop {
            let (id, response, panicked) = match self.tasks.join_next_with_id().await? {
                Ok((id, response)) => (id, response, false),
                Err(e) if e.is_panic() => {
                    tracing::error!(task = %e.id(), "dispatch handler panicked");
                    (e.id(), IpcResponse::error(500, "internal error"), true)
                }
                Err(e) => (e.id(), shutting_down(), false),
            };
            if let Some(owner) = self.owners.remove(&id) {
                return Some(Finished {
                    correlation: owner.correlation,
                    response,
                    panicked,
                });
            }
        }
    }

    /// Abort every task still running, wait for them to end, and return
    /// whom they owed an answer and what each was.
    pub(crate) async fn abort_all(&mut self) -> Vec<(Correlation, Running)> {
        let aborted = self
            .owners
            .drain()
            .map(|(_, owner)| (owner.correlation, owner.running()))
            .collect();
        self.tasks.shutdown().await;
        aborted
    }

    /// The shutdown drain: hand each request that finishes within
    /// `deadline` to `answer`, then abort the rest. Logs at info the kinds
    /// in flight when it starts and at warn each one aborted, with how long
    /// it had run; never a request's fields.
    pub(crate) async fn drain<F, Fut>(&mut self, deadline: Duration, mut answer: F) -> Drained
    where
        F: FnMut(Finished) -> Fut,
        Fut: Future<Output = ()>,
    {
        let waited = self.running();
        if waited.is_empty() {
            return Drained::default();
        }
        tracing::info!(in_flight = ?waited, "draining bus requests in flight at shutdown");
        let until = Instant::now() + deadline;
        while let Ok(Some(done)) = tokio::time::timeout_at(until, self.next()).await {
            answer(done).await;
        }
        let aborted = self.abort_all().await;
        for (_, running) in &aborted {
            tracing::warn!(
                request = running.kind,
                ran_ms = u64::try_from(running.ran.as_millis()).unwrap_or(u64::MAX),
                "bus request aborted at the shutdown drain deadline"
            );
        }
        Drained { waited, aborted }
    }
}

/// The answer for a request cut off by shutdown.
pub(crate) fn shutting_down() -> IpcResponse {
    IpcResponse::error(503, "daemon shutting down")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correlation() -> Correlation {
        Correlation {
            id: Uuid::new_v4(),
            level: SecurityLevel::Open,
        }
    }

    fn code(response: &IpcResponse) -> Option<u32> {
        match response {
            IpcResponse::Error { code, .. } => Some(*code),
            IpcResponse::Ok { .. } | IpcResponse::IntentRequired(_) => None,
        }
    }

    #[tokio::test]
    async fn a_panic_answers_500_and_the_next_request_still_runs() {
        let mut in_flight = InFlight::default();
        let panicking = correlation();
        in_flight.spawn(panicking, "Status", async { panic!("handler bug") });
        let done = in_flight.next().await.expect("one in flight");
        assert_eq!(done.correlation, panicking);
        assert_eq!(code(&done.response), Some(500));
        assert!(done.panicked);

        let healthy = correlation();
        in_flight.spawn(healthy, "Status", async { IpcResponse::ok(&"pong") });
        let done = in_flight.next().await.expect("one in flight");
        assert_eq!(done.correlation, healthy);
        assert_eq!(code(&done.response), None);
        assert!(!done.panicked);
        assert!(in_flight.next().await.is_none());
    }

    #[tokio::test]
    async fn responses_keep_their_own_correlation() {
        let mut in_flight = InFlight::default();
        let slow = correlation();
        let fast = correlation();
        in_flight.spawn(slow, "Status", async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            IpcResponse::ok(&"slow")
        });
        in_flight.spawn(fast, "Status", async { IpcResponse::ok(&"fast") });
        assert_eq!(in_flight.next().await.map(|d| d.correlation), Some(fast));
        assert_eq!(in_flight.next().await.map(|d| d.correlation), Some(slow));
    }

    #[tokio::test]
    async fn abort_all_names_every_unanswered_request() {
        let mut in_flight = InFlight::default();
        let stuck = correlation();
        in_flight.spawn(stuck, "Status", std::future::pending());
        let aborted = in_flight.abort_all().await;
        assert_eq!(
            aborted.iter().map(|(c, _)| *c).collect::<Vec<_>>(),
            vec![stuck]
        );
        assert!(in_flight.next().await.is_none());
    }

    /// Live R6-1: the drain names the request it waited out and the one it
    /// cut off, by kind and run time, and carries none of its fields.
    #[tokio::test(start_paused = true)]
    async fn the_drain_names_what_it_waited_for_and_aborted() {
        use rekindle_ipc::protocol::IpcRequest;
        let held = IpcRequest::DmSend {
            peer_key: "peer-key-hex-0123".into(),
            body: "secret message text".into(),
            reply_to: None,
        };
        let quick = IpcRequest::Status;
        let mut in_flight = InFlight::default();
        let stuck = correlation();
        let done_soon = correlation();
        in_flight.spawn(stuck, held.name(), std::future::pending());
        tokio::time::sleep(Duration::from_secs(1)).await;
        in_flight.spawn(done_soon, quick.name(), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            IpcResponse::ok(&"status")
        });

        let mut answered = Vec::new();
        let drained = in_flight
            .drain(Duration::from_secs(5), |done| {
                answered.push(done.correlation);
                std::future::ready(())
            })
            .await;

        assert_eq!(answered, vec![done_soon]);
        let kinds: Vec<_> = drained.waited.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec!["DmSend", "Status"]);
        assert_eq!(drained.aborted.len(), 1);
        let (correlation, running) = drained.aborted[0];
        assert_eq!(correlation, stuck);
        assert_eq!(running.kind, "DmSend");
        assert_eq!(running.ran, Duration::from_secs(6));
        let shown = format!("{drained:?}");
        for field in ["peer-key-hex-0123", "secret message text"] {
            assert!(
                !shown.contains(field),
                "the drain's outcome carries {field}"
            );
        }
        assert!(in_flight.next().await.is_none());
    }
}

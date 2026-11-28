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
#[path = "in_flight_tests.rs"]
mod tests;

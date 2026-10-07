//! Requests being dispatched, one task each.
//!
//! Each request runs as its own task, so a slow handler never blocks the
//! subscriber loop, and a panicking handler costs only its own request:
//! the panic surfaces here as a `JoinError` and becomes a 500 for that
//! request's correlation id, while the loop keeps serving.

use std::collections::HashMap;
use std::future::Future;

use rekindle_ipc::message::SecurityLevel;
use rekindle_ipc::protocol::IpcResponse;
use tokio::task::{Id, JoinSet};
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

/// The dispatch tasks in flight, keyed back to their requests.
#[derive(Default)]
pub(crate) struct InFlight {
    tasks: JoinSet<IpcResponse>,
    owners: HashMap<Id, Correlation>,
}

impl InFlight {
    /// Run `dispatch` as its own task, answering `correlation`.
    pub(crate) fn spawn<F>(&mut self, correlation: Correlation, dispatch: F)
    where
        F: Future<Output = IpcResponse> + Send + 'static,
    {
        let handle = self.tasks.spawn(dispatch);
        self.owners.insert(handle.id(), correlation);
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
            if let Some(correlation) = self.owners.remove(&id) {
                return Some(Finished {
                    correlation,
                    response,
                    panicked,
                });
            }
        }
    }

    /// Abort every task still running, wait for them to end, and return
    /// whom they owed an answer.
    pub(crate) async fn abort_all(&mut self) -> Vec<Correlation> {
        self.tasks.shutdown().await;
        self.owners
            .drain()
            .map(|(_, correlation)| correlation)
            .collect()
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
            IpcResponse::Ok(_) | IpcResponse::Event(_) => None,
        }
    }

    #[tokio::test]
    async fn a_panic_answers_500_and_the_next_request_still_runs() {
        let mut in_flight = InFlight::default();
        let panicking = correlation();
        in_flight.spawn(panicking, async { panic!("handler bug") });
        let done = in_flight.next().await.expect("one in flight");
        assert_eq!(done.correlation, panicking);
        assert_eq!(code(&done.response), Some(500));
        assert!(done.panicked);

        let healthy = correlation();
        in_flight.spawn(healthy, async { IpcResponse::ok(&"pong") });
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
        in_flight.spawn(slow, async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            IpcResponse::ok(&"slow")
        });
        in_flight.spawn(fast, async { IpcResponse::ok(&"fast") });
        assert_eq!(in_flight.next().await.map(|d| d.correlation), Some(fast));
        assert_eq!(in_flight.next().await.map(|d| d.correlation), Some(slow));
    }

    #[tokio::test]
    async fn abort_all_names_every_unanswered_request() {
        let mut in_flight = InFlight::default();
        let stuck = correlation();
        in_flight.spawn(stuck, std::future::pending());
        assert_eq!(in_flight.abort_all().await, vec![stuck]);
        assert!(in_flight.next().await.is_none());
    }
}

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

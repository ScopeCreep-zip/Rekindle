use super::*;

fn quiet() -> OnPanic {
    Arc::new(|_| {})
}

/// A cooperative task: runs until its scope is cancelled.
fn until_stopped(scope: &Arc<SessionScope>, name: &'static str) {
    scope
        .spawn_with_token(name, |stop| async move { stop.cancelled().await })
        .unwrap();
}

#[tokio::test]
async fn running_counts_tasks_by_name() {
    let scope = SessionScope::new("test", quiet());
    until_stopped(&scope, "b");
    until_stopped(&scope, "a");
    until_stopped(&scope, "a");
    tokio::task::yield_now().await;
    assert_eq!(scope.running(), vec![("a", 2), ("b", 1)]);
    scope.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(scope.running().is_empty());
}

/// A task runs in the span it was spawned in.
#[tokio::test]
async fn a_task_keeps_the_spawners_span() {
    let _subscriber = tracing::subscriber::set_default(tracing_subscriber::registry());
    let scope = SessionScope::new("test", quiet());
    let (tx, rx) = tokio::sync::oneshot::channel();
    tracing::info_span!("spawner").in_scope(|| {
        scope
            .spawn("named", async move {
                let name = tracing::Span::current()
                    .metadata()
                    .map(tracing::Metadata::name);
                let _ = tx.send(name);
            })
            .unwrap();
    });
    assert_eq!(rx.await.unwrap(), Some("spawner"));
}

#[tokio::test]
async fn shutdown_ends_every_task_and_refuses_new_ones() {
    let scope = SessionScope::new("test", quiet());
    for _ in 0..3 {
        until_stopped(&scope, "loop");
    }
    assert_eq!(scope.len(), 3);
    scope.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(scope.is_empty());
    assert_eq!(scope.spawn("late", async {}), Err(ScopeClosed("late")));
}

/// `close_and_wait` refuses new work and waits for one-shot work to
/// finish, however long it takes.
#[tokio::test]
async fn close_and_wait_lets_one_shot_work_finish_and_refuses_new_work() {
    let scope = SessionScope::new("test", quiet());
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
    let finished = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&finished);
    scope
        .spawn("call", async move {
            let _ = done_rx.await;
            flag.store(true, Ordering::SeqCst);
        })
        .unwrap();
    let release = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !finished.load(Ordering::SeqCst),
            "still waiting on the call"
        );
        let _ = done_tx.send(());
    };
    tokio::join!(scope.close_and_wait(), release);
    assert!(finished.load(Ordering::SeqCst));
    assert_eq!(scope.spawn("late", async {}), Err(ScopeClosed("late")));
}

/// One-shot work is not cut off mid-flight: shutdown waits for it.
#[tokio::test]
async fn one_shot_work_runs_to_completion() {
    let scope = SessionScope::new("test", quiet());
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    scope
        .spawn("send", async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            flag.store(true, Ordering::Release);
        })
        .unwrap();
    scope.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(done.load(Ordering::Acquire));
}

/// A task that ignores its token is aborted at the deadline, named,
/// and never runs again.
#[tokio::test]
async fn overdue_tasks_are_aborted_and_named() {
    let scope = SessionScope::new("test", quiet());
    let resumed = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&resumed);
    scope
        .spawn("stubborn", async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            flag.store(true, Ordering::Release);
        })
        .unwrap();
    assert_eq!(
        scope.shutdown(Duration::from_millis(50)).await,
        Err(StuckTasks(vec!["stubborn"]))
    );
    assert!(scope.is_empty());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !resumed.load(Ordering::Acquire),
        "an aborted task must not resume"
    );
}

#[tokio::test]
async fn a_panic_is_reported_once_by_name() {
    let reported = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&reported);
    let scope = SessionScope::new("test", Arc::new(move |name| sink.lock().push(name)));
    scope.spawn("first", async { panic!("bug") }).unwrap();
    scope.spawn("second", async { panic!("bug") }).unwrap();
    scope.shutdown(Duration::from_secs(1)).await.unwrap();
    let reported = reported.lock();
    assert_eq!(reported.len(), 1);
    assert!(matches!(reported[0], "first" | "second"));
}

#[tokio::test]
async fn a_parent_waits_for_and_cancels_its_children() {
    let parent = SessionScope::new("login", quiet());
    let call = parent.child("call");
    until_stopped(&call, "media");
    assert_eq!(parent.len(), 1, "the parent counts its child's tasks");
    parent.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(call.is_closed());
    assert_eq!(call.spawn("late", async {}), Err(ScopeClosed("late")));
}

#[tokio::test(start_paused = true)]
async fn stop_within_waits_out_a_task_finishing_its_call_and_aborts_none() {
    let scope = SessionScope::new("unlock", quiet());
    let landed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = Arc::clone(&landed);
    scope
        .spawn("own records", async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        })
        .unwrap();
    scope.spawn("stuck", std::future::pending()).unwrap();
    assert!(!scope.stop_within(Duration::from_secs(1)).await);
    assert!(landed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(scope.running(), vec![("stuck", 1)], "nothing aborted");
    assert_eq!(scope.spawn("late", async {}), Err(ScopeClosed("late")));
    assert_eq!(
        scope.shutdown(Duration::from_millis(10)).await,
        Err(StuckTasks(vec!["stuck"]))
    );
}

/// A parent past its deadline aborts its children's tasks too.
#[tokio::test]
async fn a_parent_aborts_overdue_child_tasks() {
    let parent = SessionScope::new("login", quiet());
    let call = parent.child("call");
    call.spawn("media", std::future::pending()).unwrap();
    assert_eq!(
        parent.shutdown(Duration::from_millis(50)).await,
        Err(StuckTasks(vec!["media"]))
    );
    assert!(call.is_empty());
}

#[tokio::test]
async fn a_child_shuts_down_alone() {
    let parent = SessionScope::new("login", quiet());
    let call = parent.child("call");
    until_stopped(&call, "media");
    until_stopped(&parent, "presence");
    call.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(!parent.is_closed());
    assert_eq!(parent.len(), 1);
    parent.shutdown(Duration::from_secs(1)).await.unwrap();
}

#[tokio::test]
async fn a_cooperative_task_runs_its_shutdown_procedure() {
    let scope = SessionScope::new("test", quiet());
    let cleaned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cleaned);
    scope
        .spawn_with_token("loop", move |stop| async move {
            stop.cancelled().await;
            tokio::task::yield_now().await; // cleanup that awaits
            flag.store(true, Ordering::Release);
        })
        .unwrap();
    scope.shutdown(Duration::from_secs(1)).await.unwrap();
    assert!(cleaned.load(Ordering::Acquire));
}

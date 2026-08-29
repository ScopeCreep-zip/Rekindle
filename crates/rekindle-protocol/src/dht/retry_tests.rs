use super::*;
use std::cell::Cell;

#[test]
fn classify_transient_vs_hard() {
    use veilid_core::VeilidAPIError as E;
    // Transient (couldn't reach holders yet) → retryable.
    for transient in [
        E::timeout(),
        E::try_again("busy"),
        E::no_connection("no route"),
        // 0.5.4+: transaction expired / lost to a concurrent open —
        // re-running the op opens a fresh transaction, so retryable.
        E::TransactionNotFound {
            message: "expired".into(),
        },
    ] {
        assert!(
            matches!(
                classify_dht_open_error("ctx", &transient),
                ProtocolError::DhtRecordUnreachable(_)
            ),
            "{transient} should be transient"
        );
    }
    // Hard error (bad input / not writable) → NOT retryable.
    assert!(matches!(
        classify_dht_open_error("ctx", &E::generic("not writable")),
        ProtocolError::DhtError(_)
    ));
}

#[tokio::test]
async fn retry_recovers_after_transient_failures() {
    let calls = Cell::new(0u32);
    let res: Result<&str, ProtocolError> =
        retry_on_unreachable(5, std::time::Duration::ZERO, || {
            let n = calls.get();
            calls.set(n + 1);
            async move {
                if n < 2 {
                    Err(ProtocolError::DhtRecordUnreachable(format!(
                        "transient {n}"
                    )))
                } else {
                    Ok("opened")
                }
            }
        })
        .await;
    assert_eq!(res.unwrap(), "opened");
    assert_eq!(
        calls.get(),
        3,
        "failed twice, succeeded on the third attempt"
    );
}

#[tokio::test]
async fn retry_exhausts_then_surfaces_error() {
    let calls = Cell::new(0u32);
    let res: Result<(), ProtocolError> = retry_on_unreachable(3, std::time::Duration::ZERO, || {
        calls.set(calls.get() + 1);
        async { Err(ProtocolError::DhtRecordUnreachable("still gone".into())) }
    })
    .await;
    assert!(matches!(res, Err(ProtocolError::DhtRecordUnreachable(_))));
    assert_eq!(calls.get(), 3, "uses the full budget before giving up");
}

#[tokio::test]
async fn retry_does_not_retry_hard_errors() {
    let calls = Cell::new(0u32);
    let res: Result<(), ProtocolError> = retry_on_unreachable(5, std::time::Duration::ZERO, || {
        calls.set(calls.get() + 1);
        async { Err(ProtocolError::DhtError("not writable".into())) }
    })
    .await;
    assert!(matches!(res, Err(ProtocolError::DhtError(_))));
    assert_eq!(
        calls.get(),
        1,
        "a hard error must NOT be retried (no key churn delay)"
    );
}

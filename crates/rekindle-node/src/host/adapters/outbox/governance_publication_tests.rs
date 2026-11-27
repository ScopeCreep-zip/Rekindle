use super::*;

fn change(state: SendState, reason: Option<&str>) -> StateChange {
    StateChange {
        stream: "governance_op:VLD0:a".into(),
        stream_seq: 1,
        client_id: "ab".into(),
        kind: StreamKind::GovernanceOp,
        state,
        wedge_reason: reason.map(str::to_owned),
        retry: None,
    }
}

#[test]
fn a_stored_row_is_pending_and_a_failed_one_failed_with_its_reason() {
    assert_eq!(
        announced(&change(SendState::Queued, None)),
        Some(OpPublicationState::Pending)
    );
    assert_eq!(
        announced(&change(SendState::Failed, Some("past its deadline"))),
        Some(OpPublicationState::Failed("past its deadline".into()))
    );
    assert_eq!(
        announced(&change(SendState::Failed, None)),
        Some(OpPublicationState::Failed("its row was discarded".into()))
    );
    assert_eq!(announced(&change(SendState::Sent, None)), None);
    assert_eq!(community_of("governance_op:VLD0:a"), Some("VLD0:a"));
}

fn row(state: SendState, reason: Option<&str>) -> OutboxRow {
    OutboxRow {
        stream: "governance_op:VLD0:a".into(),
        stream_seq: 1,
        client_id: "ab".into(),
        kind: StreamKind::GovernanceOp,
        payload: Vec::new(),
        state,
        attempts: 0,
        next_at: 0,
        deadline: 0,
        wedge_reason: reason.map(str::to_owned),
        sent_at: None,
        depends_on: None,
    }
}

/// A window opened after an op's `Pending` reads it from its row
/// (walk M86: a window that missed the event kept no state for it).
#[test]
fn a_waiting_row_reads_pending_a_failed_one_failed_and_a_sent_one_nothing() {
    assert_eq!(
        row_state(&row(SendState::Queued, None)),
        Some(OpPublicationState::Pending)
    );
    assert_eq!(
        row_state(&row(SendState::Failed, Some("past its deadline"))),
        Some(OpPublicationState::Failed("past its deadline".into()))
    );
    assert_eq!(row_state(&row(SendState::Sent, None)), None);
}

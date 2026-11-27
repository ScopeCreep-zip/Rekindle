//! The held set (plan E3.2 S7d-keys.10): every unexpired review held for
//! exactly one owner, under the id it was first published with.

use super::*;

fn ids(held: &[(IntentId, IntentReview)]) -> Vec<(IntentId, IntentId)> {
    held.iter().map(|(hold, r)| (*hold, r.intent_id)).collect()
}

#[test]
fn a_held_review_keeps_its_publication_id_until_consumed() {
    let gate = gate();
    let initial = selection(IntentKind::BlockUser, vec![]);
    let first = gate
        .hold(ME, initial.clone(), "original", None, 10, 0)
        .unwrap();
    assert_eq!(
        ids(&gate.held_reviews(&ME, 0)),
        vec![(first.intent_id, first.intent_id)]
    );
    assert!(gate.held_reviews(&who(2), 0).is_empty(), "another owner's");
    assert!(gate.held_reviews(&ME, 10).is_empty(), "expired");

    let mut changed = initial;
    changed.facts = vec![Fact::MessagesDeleted { count: 11 }];
    let Ok(Gate::Review(refreshed)) = gate.commit(
        first.intent_id,
        &ME,
        &IntentProof::Commit,
        changed.clone(),
        no_passphrase,
        1,
    ) else {
        panic!("changed facts must be reviewed again");
    };
    assert_ne!(first.intent_id, refreshed.intent_id);
    assert_eq!(
        ids(&gate.held_reviews(&ME, 1)),
        vec![(first.intent_id, refreshed.intent_id)],
        "a refresh keeps the publication id"
    );
    assert!(matches!(
        gate.commit(
            refreshed.intent_id,
            &ME,
            &IntentProof::Commit,
            changed.clone(),
            no_passphrase,
            2
        ),
        Ok(Gate::Run("original"))
    ));
    assert!(gate.held_reviews(&ME, 2).is_empty(), "committed");

    let cancelled = gate
        .hold(ME, changed.clone(), "cancel", None, 10, 2)
        .unwrap();
    gate.cancel(cancelled.intent_id, &ME).unwrap();
    assert!(gate.held_reviews(&ME, 2).is_empty(), "cancelled");

    gate.hold(ME, changed.clone(), "replace", None, 10, 2)
        .unwrap();
    gate.retain_held(|owner, action| *owner != ME || *action != "replace");
    let latest = gate.hold(ME, changed, "replace", None, 10, 3).unwrap();
    assert_eq!(
        ids(&gate.held_reviews(&ME, 3)),
        vec![(latest.intent_id, latest.intent_id)],
        "a replacement supersedes"
    );
}

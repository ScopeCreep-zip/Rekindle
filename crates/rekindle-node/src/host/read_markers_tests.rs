use super::*;

#[test]
fn a_pending_marker_only_rises() {
    let pending = PendingMarkers::default();
    let scope = Scope::Dm {
        peer_key: "p".into(),
    };
    pending.note(scope.clone(), 4);
    pending.note(scope.clone(), 2);
    assert_eq!(pending.get(&scope), Some(4));
    assert_eq!(
        pending.take().into_iter().collect::<Vec<_>>(),
        vec![(scope.clone(), 4)]
    );
    assert_eq!(pending.get(&scope), None);
}

use super::WatchManager;

#[test]
fn tracks_active_records() {
    let mut manager = WatchManager::default();
    manager.mark_active("gov");
    manager.mark_active("chan");
    assert!(manager.is_active("gov"));
    manager.mark_inactive("gov");
    assert!(!manager.is_active("gov"));
    assert_eq!(manager.len(), 1);
}

use super::*;

#[test]
fn insert_remove_contains() {
    let mut set = PinnedSet::new();
    let id = Uuid::new_v4();
    assert!(!set.contains(&id));
    set.insert(id);
    assert!(set.contains(&id));
    assert_eq!(set.len(), 1);
    set.remove(&id);
    assert!(!set.contains(&id));
    assert!(set.is_empty());
}

#[test]
fn replace_overwrites() {
    let mut set = PinnedSet::new();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    set.insert(a);
    set.replace([b]);
    assert!(!set.contains(&a));
    assert!(set.contains(&b));
}

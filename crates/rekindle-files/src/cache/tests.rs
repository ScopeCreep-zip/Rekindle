use super::*;
use tempfile::TempDir;

const KEY: [u8; 32] = [7u8; 32];

fn cache(temp: &TempDir, budget: u64) -> ChunkCache {
    open_at(temp.path().to_path_buf(), budget, KEY)
}

fn open_at(root_dir: PathBuf, budget: u64, key: [u8; 32]) -> ChunkCache {
    ChunkCache::open(CacheConfig {
        root_dir,
        byte_budget: budget,
        name_key: Zeroizing::new(key),
    })
    .unwrap()
}

/// Every file and directory name under `root`, recursively.
fn names_under(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        out.push(entry.file_name().to_string_lossy().into_owned());
        if entry.file_type().unwrap().is_dir() {
            out.extend(names_under(&entry.path()));
        }
    }
    out
}

#[test]
fn insert_and_get_roundtrip() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 1024 * 1024);
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    c.insert(id, 0, b"hello", &pinned).unwrap();
    assert_eq!(c.get(id, 0).unwrap().as_deref(), Some(&b"hello"[..]));
    assert_eq!(c.total_bytes(), 5);
}

#[test]
fn missing_chunk_returns_none() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 1024);
    let id = Uuid::new_v4();
    assert!(c.get(id, 0).unwrap().is_none());
}

#[test]
fn lru_evicts_when_over_budget() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 30); // tight budget
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    c.insert(id, 0, &[0u8; 10], &pinned).unwrap();
    c.insert(id, 1, &[0u8; 10], &pinned).unwrap();
    c.insert(id, 2, &[0u8; 10], &pinned).unwrap();
    // total = 30, exactly at budget; no eviction yet.
    assert_eq!(c.total_bytes(), 30);
    c.insert(id, 3, &[0u8; 10], &pinned).unwrap();
    // Inserted +10 → 40 → exceeded → evicted oldest until ≤ 30.
    assert!(c.total_bytes() <= 30);
    // Most recent should still be present.
    assert!(c.get(id, 3).unwrap().is_some());
}

#[test]
fn pinned_chunks_survive_eviction() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 25);
    let mut pinned = PinnedSet::new();
    let pinned_id = Uuid::new_v4();
    let other_id = Uuid::new_v4();
    pinned.insert(pinned_id);

    c.insert(pinned_id, 0, &[0u8; 10], &pinned).unwrap();
    c.insert(pinned_id, 1, &[0u8; 10], &pinned).unwrap();
    c.insert(other_id, 0, &[0u8; 10], &pinned).unwrap();
    // 30 bytes > 25 → evict — must skip the pinned ones.
    assert!(c.get(pinned_id, 0).unwrap().is_some());
    assert!(c.get(pinned_id, 1).unwrap().is_some());
    assert!(c.get(other_id, 0).unwrap().is_none());
}

#[test]
fn bitmap_reflects_inserts() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 1024);
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    c.insert(id, 0, b"a", &pinned).unwrap();
    c.insert(id, 3, b"b", &pinned).unwrap();
    c.insert(id, 7, b"c", &pinned).unwrap();
    let bm = c.bitmap_for(id, 8).unwrap();
    assert!(bm.has(0));
    assert!(bm.has(3));
    assert!(bm.has(7));
    assert!(!bm.has(1));
    assert_eq!(bm.count(), 3);
}

#[test]
fn open_rebuilds_from_disk() {
    let temp = TempDir::new().unwrap();
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    {
        let mut c = cache(&temp, 1024);
        c.insert(id, 0, b"persisted", &pinned).unwrap();
        c.insert(id, 5, b"more", &pinned).unwrap();
    }
    // Re-open and confirm chunks are still discoverable.
    let mut c2 = cache(&temp, 1024);
    assert_eq!(c2.total_bytes(), 9 + 4);
    assert_eq!(c2.get(id, 0).unwrap().as_deref(), Some(&b"persisted"[..]));
    assert_eq!(c2.get(id, 5).unwrap().as_deref(), Some(&b"more"[..]));
}

#[test]
fn remove_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let mut c = cache(&temp, 1024);
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    c.insert(id, 0, b"x", &pinned).unwrap();
    c.remove(id, 0).unwrap();
    c.remove(id, 0).unwrap(); // second remove is fine
    assert_eq!(c.total_bytes(), 0);
    assert!(c.get(id, 0).unwrap().is_none());
}

#[test]
fn no_name_on_disk_carries_a_scope_or_attachment_id() {
    let temp = TempDir::new().unwrap();
    let files = temp.path().join("pk.files");
    let community = "VLD0:community-record-key";
    let conversation = "ab".repeat(32);
    let id = Uuid::new_v4();
    let pinned = PinnedSet::new();
    for scope in [
        CacheScope::Community(community),
        CacheScope::Dm(&conversation),
        CacheScope::CommunityAvatars(community),
    ] {
        let mut c = open_at(scope_root(&files, &KEY, scope), 1024, KEY);
        c.insert(id, 0, b"x", &pinned).unwrap();
    }
    let names = names_under(&files);
    let id_hex = hex::encode(id.as_bytes());
    for name in &names {
        for raw in [community, conversation.as_str(), id_hex.as_str()] {
            assert!(!name.contains(raw), "{name} names {raw}");
        }
        assert!(!name.contains(&id.to_string()), "{name}");
    }
    // Three scopes, each with one attachment directory and one chunk.
    let dirs: Vec<_> = names.iter().filter(|n| n.as_str() != "0.bin").collect();
    assert_eq!(dirs.len(), 6);
    assert!(dirs
        .iter()
        .all(|n| n.len() == names::NAME_HEX_LEN && AttachmentName::parse(n).is_some()));
    // One file shared in two scopes is two different directory names.
    let distinct: std::collections::HashSet<_> = dirs.iter().collect();
    assert_eq!(distinct.len(), 6);
}

#[test]
fn names_are_stable_across_reopen_and_unlinkable_across_identities() {
    let temp = TempDir::new().unwrap();
    let files = temp.path().join("pk.files");
    let scope = CacheScope::Community("community");
    let root = scope_root(&files, &KEY, scope);
    assert_eq!(root, scope_root(&files, &KEY, scope));
    let other_key = [8u8; 32];
    assert_ne!(root, scope_root(&files, &other_key, scope));
    assert_ne!(
        root,
        scope_root(&files, &KEY, CacheScope::CommunityAvatars("community"))
    );

    let id = Uuid::new_v4();
    let first = open_at(root.clone(), 1024, KEY).attachment_name(id);
    assert_eq!(first, open_at(root.clone(), 1024, KEY).attachment_name(id));
    assert_ne!(first, open_at(root, 1024, other_key).attachment_name(id));
}

#[test]
fn the_open_walk_keys_the_lru_by_hashed_name_and_evicts_across_reopen() {
    let temp = TempDir::new().unwrap();
    let root = scope_root(temp.path(), &KEY, CacheScope::Dm("conversation"));
    let mut pinned = PinnedSet::new();
    let (keep, old, new) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    pinned.insert(keep);
    {
        let mut c = open_at(root.clone(), 1024, KEY);
        c.insert(keep, 0, &[0u8; 10], &pinned).unwrap();
        c.insert(old, 0, &[0u8; 10], &pinned).unwrap();
        c.insert(old, 1, &[0u8; 10], &pinned).unwrap();
    }
    // A stray directory that is no keyed name is ignored.
    std::fs::create_dir_all(root.join("not-a-name")).unwrap();
    let mut c = open_at(root.clone(), 25, KEY);
    assert_eq!(c.total_bytes(), 30);
    assert_eq!(c.chunk_count(), 3);
    assert!(c.holds(old) && c.holds(keep) && !c.holds(new));
    assert_eq!(c.bitmap_for(old, 2).unwrap().count(), 2);
    // Over budget: the walk-seeded entries go first, the pinned one stays.
    c.insert(new, 0, &[0u8; 15], &pinned).unwrap();
    assert_eq!(c.total_bytes(), 25);
    assert!(c.holds(keep));
    assert!(c.get(new, 0).unwrap().is_some());
    assert!(!c.holds(old));

    // Under a different key the same tree holds nothing this identity names.
    let foreign = open_at(root, 1024, [9u8; 32]);
    assert!(!foreign.holds(keep));
}

#[test]
fn an_emptied_attachment_directory_is_removed_on_open() {
    let temp = TempDir::new().unwrap();
    let pinned = PinnedSet::new();
    let id = Uuid::new_v4();
    let dir = {
        let mut c = cache(&temp, 1024);
        c.insert(id, 0, b"x", &pinned).unwrap();
        c.remove(id, 0).unwrap();
        temp.path().join(c.attachment_name(id).to_hex())
    };
    assert!(dir.exists());
    let _ = cache(&temp, 1024);
    assert!(!dir.exists());
}

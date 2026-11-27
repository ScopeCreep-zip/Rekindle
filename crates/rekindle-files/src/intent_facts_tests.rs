use super::*;

#[test]
fn facts_name_the_file_and_say_metadata_is_not_removed() {
    let dir = std::env::temp_dir().join(format!("rk-facts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("photo.jpg");
    std::fs::write(&path, [0u8; 10]).unwrap();
    let facts = attachment_facts(&path).unwrap();
    assert_eq!(facts.file_name, "photo.jpg");
    assert_eq!(facts.size_bytes, 10);
    assert!(!facts.metadata_removed);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(attachment_facts_for_bytes("a.png", &[1, 2]).size_bytes, 2);
}

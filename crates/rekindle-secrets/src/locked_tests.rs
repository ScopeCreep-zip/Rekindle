use super::{LockedBox, PAGE_ALIGN};

#[test]
fn the_secret_sits_alone_on_locked_pages() {
    let secret = LockedBox::<32>::new_with(|buf| buf.fill(7));
    assert_eq!(secret.expose(), &[7u8; 32]);
    assert!(secret.is_locked(), "a 16 KiB lock fits any default budget");
    let address = secret.expose().as_ptr() as usize;
    assert_eq!(address % PAGE_ALIGN, 0, "the buffer starts a page");
    assert_eq!(std::mem::size_of::<super::Pages<32>>(), PAGE_ALIGN);
    let page = region::query(secret.expose().as_ptr()).expect("query");
    assert!(page.is_committed());
}

#[test]
fn debug_shows_no_bytes() {
    let secret = LockedBox::<4>::from_bytes(&[0xAB; 4]);
    let shown = format!("{secret:?}");
    assert!(
        !shown.contains("171") && !shown.to_lowercase().contains("ab,"),
        "{shown}"
    );
}

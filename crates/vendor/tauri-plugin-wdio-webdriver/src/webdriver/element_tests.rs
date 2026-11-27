use super::*;

#[test]
fn test_store_element() {
    let mut store = ElementStore::new();
    let elem = store.store();

    assert!(!elem.id.is_empty());
    assert!(elem.js_ref.starts_with("__wd_el_"));
    // js_ref uses ID without hyphens
    assert!(elem.js_ref.contains(&elem.id.replace('-', "")));
}

#[test]
fn test_get_element() {
    let mut store = ElementStore::new();
    let elem = store.store();
    let id = elem.id.clone();

    let retrieved = store.get(&id).expect("element should exist");
    assert_eq!(retrieved.id, id);
}

#[test]
fn test_js_ref_uses_id_without_hyphens() {
    let mut store = ElementStore::new();
    let elem1 = store.store();
    let elem2 = store.store();

    // js_ref should use ID with hyphens removed for valid JS variable name
    assert_eq!(
        elem1.js_ref,
        format!("__wd_el_{}", elem1.id.replace('-', ""))
    );
    assert_eq!(
        elem2.js_ref,
        format!("__wd_el_{}", elem2.id.replace('-', ""))
    );
}

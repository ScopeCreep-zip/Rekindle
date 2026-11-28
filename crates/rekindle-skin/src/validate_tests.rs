use super::*;

const TOTAL_CHAOS: &[u8] = include_bytes!("../builtin/total-chaos.rkskin");

#[test]
fn an_import_trial_accepts_the_pixels_that_the_final_pack_keeps() {
    check_trial(TOTAL_CHAOS, ENTRIES).unwrap();
    assert_eq!(run(TOTAL_CHAOS).unwrap().bytes, TOTAL_CHAOS);
}

#[test]
fn an_import_trial_refuses_the_same_bad_png_as_the_final_validator() {
    let source = package::read(TOTAL_CHAOS).unwrap();
    let mut files: BTreeMap<String, Vec<u8>> = source
        .files()
        .map(|(name, bytes)| (name.to_owned(), bytes.to_vec()))
        .collect();
    let (name, png) = files
        .iter_mut()
        .find(|(name, _)| name.starts_with("assets/") && name.ends_with(".png"))
        .unwrap();
    let name = name.clone();
    png.truncate(8);
    let broken = output::zip(&files).unwrap();
    let refusal = run(&broken).unwrap_err();
    assert_eq!(refusal.entry, name);
    assert_eq!(check_trial(&broken, ENTRIES), Err(refusal));
}

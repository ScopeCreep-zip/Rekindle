//! Every built-in sets every dialog role from its own colours (plan
//! E7.7.3 item 1, E7.7.7): a pack's named colour or the documented
//! derivation over the colours it defines, never a registry default.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rekindle_skin::import::dialog_roles::DERIVE;
use rekindle_skin::registry::DIALOG;
use rekindle_skin::{builtin, manifest, package, tokens};
use rekindle_types::dto::skin::TokenValue;

fn tokens_of(id: &str) -> BTreeMap<String, TokenValue> {
    let pack = builtin::PACKS.iter().find(|p| p.id == id).unwrap();
    let pkg = package::read(pack.bytes).unwrap();
    let m = manifest::parse(&pkg).unwrap();
    let v = &m.variants[0];
    tokens::parse(&v.tokens, pkg.get(&v.tokens).unwrap()).unwrap()
}

fn rgb(t: &BTreeMap<String, TokenValue>, id: &str) -> [u8; 3] {
    match &t[id] {
        TokenValue::Color { r, g, b, .. } => [*r, *g, *b],
        v => panic!("{id}: {v:?}"),
    }
}

/// Why a built-in's role can be unset: a pack whose source tree is not in
/// `legacy/skins/<id>/` (the README lists it) cannot be re-imported, so it
/// keeps the roles of its last import. The row stays red until its source
/// is found; the reason is read from the tree, never a list here.
fn unset_reason(id: &str) -> &'static str {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/skins").join(id);
    if tree.is_dir() {
        "its source is in legacy/skins: re-import it (a refused import names the pair the walk cannot settle)"
    } else {
        "no source tree in legacy/skins (see its README), so it cannot be re-imported"
    }
}

/// Includes `input.text`: with `TextFieldTextCG` unmapped no XML pack's
/// field text is its own (see the import unit test for the proof).
#[test]
fn every_builtin_sets_every_dialog_role() {
    let mut unset = Vec::new();
    for pack in &builtin::PACKS {
        let t = tokens_of(pack.id);
        let roles = DERIVE.iter().map(|(id, _)| *id).chain(DIALOG.iter().map(|e| e.id));
        let missing: BTreeSet<&str> = roles.filter(|id| !t.contains_key(*id)).collect();
        if !missing.is_empty() {
            unset.push(format!("{}: {missing:?} unset: {}", pack.id, unset_reason(pack.id)));
        }
    }
    assert!(unset.is_empty(), "{unset:#?}");
}

/// Symbiosis's `Themes.xml` (lines 16, 27, 29, 48-53).
#[test]
fn symbiosis_takes_its_own_dialog_colours() {
    let t = tokens_of("symbiosis");
    for (id, want) in [
        ("input.bg", [30, 30, 30]),
        ("input.text", [255, 255, 255]),
        ("input.selection.bg", [0, 109, 166]),
        ("menubar.text", [143, 185, 215]),
        ("text.disabled", [180, 180, 180]),
        ("tooltip.bg", [56, 56, 56]),
        ("tooltip.text", [231, 231, 231]),
        ("selection.text", [255, 255, 255]),
    ] {
        assert_eq!(rgb(&t, id), want, "{id}");
    }
}

fn walked(notice: &str, id: &str) -> bool {
    notice.lines().any(|l| l.starts_with(&format!("  {id}: #")))
}

fn derived(t: &BTreeMap<String, TokenValue>, id: &str) -> Option<TokenValue> {
    let (_, d) = DERIVE.iter().find(|(r, _)| *r == id)?;
    let view: BTreeMap<&str, TokenValue> = t.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    d.apply(&view)
}

/// The title ground of a pack with a strip is its strip's, not the
/// strip-less derivation (Symbiosis's charcoal strip, not its blue accent).
#[test]
fn symbiosis_title_ground_comes_from_its_strip() {
    let t = tokens_of("symbiosis");
    for id in ["titlebar.start", "titlebar.end"] {
        assert_ne!(Some(t[id].clone()), derived(&t, id), "{id}");
    }
}

/// The INI packs name no text field, selection, progress well or thumb
/// colour: each is the derivation over the pack's own colours, unless the
/// walk moved it or a source (recorded in the NOTICE).
#[test]
fn ini_packs_derive_the_roles_they_do_not_name() {
    for (id, notice) in [
        ("alloy", include_str!("../builtin/alloy.NOTICE")),
        ("combat", include_str!("../builtin/combat.NOTICE")),
        ("simple-x", include_str!("../builtin/simple-x.NOTICE")),
        ("xglow", include_str!("../builtin/xglow.NOTICE")),
    ] {
        let t = tokens_of(id);
        for role in [
            "input.bg",
            "input.text",
            "input.selection.bg",
            "input.selection.bg-inactive",
            "progress.bg",
            "progress.outline",
            "progress.text",
            "scrollbar.thumb",
        ] {
            let (_, d) = DERIVE.iter().find(|(r, _)| *r == role).unwrap();
            let moved = walked(notice, role) || d.sources().iter().any(|s| walked(notice, s));
            assert!(
                moved || Some(t[role].clone()) == derived(&t, role),
                "{id}: {role}"
            );
        }
    }
}

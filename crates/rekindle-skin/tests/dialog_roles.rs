//! Every built-in sets every dialog role from its own colours (plan
//! E7.7.3 item 1, E7.7.7): a pack's named colour or art, or the registry's
//! one derivation table over the colours it defines (ST1 ruling 4), never a
//! registry default.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rekindle_skin::registry::{dialog, entry_in, resolve, Base, ENTRIES};
use rekindle_skin::{builtin, manifest, package, tokens};
use rekindle_types::dto::skin::TokenValue;

fn variant_of(id: &str) -> (BTreeMap<String, TokenValue>, Base) {
    let pack = builtin::PACKS.iter().find(|p| p.id == id).unwrap();
    let pkg = package::read(pack.bytes).unwrap();
    let m = manifest::parse(&pkg).unwrap();
    let v = &m.variants[0];
    let t = tokens::parse(&v.tokens, pkg.get(&v.tokens).unwrap()).unwrap();
    (t, Base::of(v.base, v.contrast))
}

fn tokens_of(id: &str) -> BTreeMap<String, TokenValue> {
    variant_of(id).0
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
    // xfire-classic has no source tree: load serves its roles (below).
    for pack in builtin::PACKS.iter().filter(|p| p.id != "xfire-classic") {
        let t = tokens_of(pack.id);
        let missing: BTreeSet<&str> = dialog::roles().filter(|id| !t.contains_key(*id)).collect();
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

/// The registry's derivation of `id` over what `t` serves.
fn derived(t: &BTreeMap<String, TokenValue>, base: Base, id: &str) -> Option<TokenValue> {
    let d = entry_in(ENTRIES, id)?.derivation?;
    let served = resolve(ENTRIES, base, t);
    let view: BTreeMap<&str, TokenValue> = served.into_iter().map(|(e, v)| (e.id, v)).collect();
    d.apply(&view)
}

/// xfire-classic has no source tree and was imported before the roles
/// existed: `registry::resolve` serves every role at load by the same table
/// the importer writes from (ST1 ruling 4), so none takes a default.
/// Fail-without: resolve's dialog rows following only a token the skin
/// sets serves `menubar.bg` the default #0a0a0a, not its #060c1c.
#[test]
fn xfire_classic_is_served_every_dialog_role_from_its_own_colours() {
    let (t, base) = variant_of("xfire-classic");
    let served = resolve(ENTRIES, base, &t);
    let view: BTreeMap<&str, TokenValue> =
        served.iter().map(|(e, v)| (e.id, v.clone())).collect();
    for id in dialog::roles() {
        assert!(!t.contains_key(id), "{id}: its token file sets it");
        assert_eq!(Some(view[id].clone()), derived(&t, base, id), "{id}");
    }
}

/// The title ground of a pack with a strip is its strip's, not the
/// strip-less derivation (Symbiosis's charcoal strip, not its blue accent).
#[test]
fn symbiosis_title_ground_comes_from_its_strip() {
    let (t, base) = variant_of("symbiosis");
    for id in ["titlebar.start", "titlebar.end"] {
        let mut strip_less = t.clone();
        strip_less.remove(id);
        assert_ne!(Some(t[id].clone()), derived(&strip_less, base, id), "{id}");
    }
}

/// The INI packs name no text field, selection, progress well or thumb
/// colour: each is the derivation over the pack's settled colours (ST1
/// ruling 2), unless the dialog walk moved it or a dialog role it reads
/// (recorded in the NOTICE). A base colour never moves for them.
#[test]
fn ini_packs_derive_the_roles_they_do_not_name() {
    for (id, notice) in [
        ("alloy", include_str!("../builtin/alloy.NOTICE")),
        ("combat", include_str!("../builtin/combat.NOTICE")),
        ("simple-x", include_str!("../builtin/simple-x.NOTICE")),
        ("xglow", include_str!("../builtin/xglow.NOTICE")),
    ] {
        let (t, base) = variant_of(id);
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
            let d = entry_in(ENTRIES, role).unwrap().derivation.unwrap();
            let moved = walked(notice, role)
                || d.sources().iter().any(|s| dialog::is_role(s) && walked(notice, s));
            let mut unset = t.clone();
            unset.remove(role);
            assert!(
                moved || Some(t[role].clone()) == derived(&unset, base, role),
                "{id}: {role}"
            );
        }
    }
}

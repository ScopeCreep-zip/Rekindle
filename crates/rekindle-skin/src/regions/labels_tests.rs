//! Every label slot's backing over its own art (plan E7.7.7, V2).

use super::*;
use crate::serve::active_resolved;

const BUILTINS: [&str; 11] = [
    "symbiosis",
    "xfire-classic",
    "combat",
    "onyx",
    "spacecowboy",
    "stealthx",
    "total-chaos",
    "xslimfire",
    "alloy",
    "simple-x",
    "xglow",
];

/// Built-in `b`'s label grounds and the label CSS its skin layer carries.
fn builtin_labels(b: &str) -> (BTreeMap<String, TitleGround>, String) {
    let (_, builtin) =
        crate::builtin::find(&format!("{}{b}", crate::builtin::PREFIX)).expect("a built-in");
    let p = crate::package::read(builtin.bytes).expect("package");
    let m = crate::manifest::parse(&p).expect("manifest");
    let (_, resolved) = active_resolved(&p, &m).expect("tokens");
    let images: BTreeMap<String, Vec<u8>> =
        p.files().map(|(n, b)| (n.to_owned(), b.to_vec())).collect();
    (
        label_grounds(&m, &images, &resolved),
        label_css(&m, &images, &resolved),
    )
}

#[test]
fn the_skin_layer_drops_a_label_backing_only_where_the_art_carries_it() {
    for b in BUILTINS {
        let (grounds, css) = builtin_labels(b);
        for (id, g) in &grounds {
            let decl = format!("{}: transparent;", backing_property(id));
            assert_eq!(g.bare(), css.contains(&decl), "{b} {id} {g:?}");
        }
        assert_eq!(
            css.is_empty(),
            !grounds.values().any(TitleGround::bare),
            "{b}"
        );
    }
    // The served stylesheet carries it after the regions it overrides.
    let (_, builtin) = crate::builtin::find("builtin:symbiosis").expect("a built-in");
    let sheet = crate::serve::stylesheet(builtin.bytes, &"0".repeat(64)).expect("sheet");
    let decl = format!("{}: transparent;", backing_property("statusbar"));
    assert!(sheet.css.contains(&decl));
}

#[test]
fn each_builtin_drops_the_label_backings_its_art_carries() {
    let got: Vec<String> = BUILTINS
        .iter()
        .map(|b| {
            let (grounds, _) = builtin_labels(b);
            let parts: Vec<String> = grounds
                .iter()
                .map(|(id, g)| {
                    let mark = if g.bare() { "bare" } else { "kept" };
                    format!("{id}={}:{mark}", g.worst_x100)
                })
                .collect();
            format!("{b}: {}", parts.join(" "))
        })
        .collect();
    // Worst pixel x100 per labelled slot (SC 1.4.3 floor 450): Xfire
    // Classic's header art keeps every backing (its worst pixels, over a
    // white desktop or its light bevels, are 1.00-2.00:1 under its label
    // colours).
    let want = [
        "symbiosis: buddylist.actionbar=208:kept buddylist.group-header=680:bare buddylist.infobar=128:kept buddylist.menubar=836:bare chat.group-header=680:bare chat.input=327:kept chat.toolbar=185:kept community.category-header=680:bare community.channel-header=208:kept community.member-group-header=680:bare community.sidebar-header=137:kept statusbar=588:bare",
        "xfire-classic: buddylist.actionbar=100:kept buddylist.group-header=200:kept buddylist.infobar=124:kept buddylist.menubar=100:kept chat.group-header=200:kept chat.toolbar=100:kept community.category-header=200:kept community.channel-header=100:kept community.member-group-header=200:kept community.sidebar-header=124:kept statusbar=122:kept",
        "combat: buddylist.group-header=200:kept buddylist.infobar=100:kept chat.group-header=102:kept chat.toolbar=100:kept community.category-header=200:kept community.channel-header=100:kept community.member-group-header=200:kept community.sidebar-header=100:kept",
        "onyx: buddylist.actionbar=208:kept buddylist.group-header=350:kept buddylist.menubar=123:kept community.category-header=350:kept community.member-group-header=350:kept statusbar=123:kept",
        "spacecowboy: buddylist.group-header=496:bare community.category-header=496:bare community.member-group-header=496:bare statusbar=123:kept",
        "stealthx: buddylist.group-header=501:bare buddylist.menubar=466:bare chat.group-header=501:bare community.category-header=501:bare community.member-group-header=501:bare",
        "total-chaos: buddylist.actionbar=100:kept buddylist.group-header=277:kept buddylist.menubar=100:kept community.category-header=277:kept community.member-group-header=277:kept",
        "xslimfire: buddylist.group-header=773:bare chat.group-header=773:bare community.category-header=773:bare community.member-group-header=773:bare statusbar=447:kept",
        "alloy: buddylist.group-header=234:kept buddylist.infobar=164:kept community.category-header=234:kept community.member-group-header=234:kept community.sidebar-header=183:kept statusbar=100:kept",
        "simple-x: buddylist.group-header=200:kept buddylist.infobar=100:kept community.category-header=200:kept community.member-group-header=200:kept community.sidebar-header=100:kept statusbar=100:kept",
        "xglow: buddylist.group-header=110:kept community.category-header=110:kept community.member-group-header=110:kept",
    ];
    assert_eq!(got, want);
}

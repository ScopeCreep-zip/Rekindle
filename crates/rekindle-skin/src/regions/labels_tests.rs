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
fn the_skin_layer_sets_a_band_only_where_the_art_fails_its_labels() {
    for b in BUILTINS {
        let (grounds, css) = builtin_labels(b);
        for (id, g) in &grounds {
            let n = css.matches(&format!("{}:", band_property(id))).count();
            assert_eq!(n, usize::from(!g.bare()), "{b} {id} {g:?}");
        }
        assert_eq!(
            css.is_empty(),
            grounds.values().all(TitleGround::bare),
            "{b}"
        );
    }
}

/// Symbiosis's status bar art meets 4.5:1 under its labels (6.00:1): the
/// served sheet sets no band for it, so nothing is drawn behind its text.
#[test]
fn a_label_over_art_that_meets_its_floor_serves_no_backing() {
    let (_, builtin) = crate::builtin::find("builtin:symbiosis").expect("a built-in");
    let sheet = crate::serve::stylesheet(builtin.bytes, &"0".repeat(64)).expect("sheet");
    assert!(
        !sheet.css.contains(&band_property("statusbar")),
        "{}",
        sheet.css
    );
}

/// Symbiosis's community sidebar header art fails under its labels
/// (1.42:1): the served sheet sets its band exactly once, to the slot's
/// backing, which the container paints once across its whole strip.
#[test]
fn a_label_over_failing_art_serves_one_band_for_the_region() {
    let (_, builtin) = crate::builtin::find("builtin:symbiosis").expect("a built-in");
    let sheet = crate::serve::stylesheet(builtin.bytes, &"0".repeat(64)).expect("sheet");
    let id = "community.sidebar-header";
    let decl = format!("{}: var({});", band_property(id), backing_property(id));
    assert_eq!(
        sheet.css.matches(&band_property(id)).count(),
        1,
        "{}",
        sheet.css
    );
    assert!(sheet.css.contains(&decl), "{}", sheet.css);
}

#[test]
fn each_builtin_bands_only_the_labelled_slots_its_art_fails() {
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
    // Worst pixel x100 under each labelled slot's labels (SC 1.4.3 floor
    // 450, 300 large); `kept` is a slot whose band the skin layer sets (one
    // band behind its text row), `bare` one with none. Xfire Classic's
    // header art keeps every band (its
    // worst pixels under the label rows, over a white desktop or its light
    // fill, are 1.00-4.06:1 under its label colours).
    let want = [
        "symbiosis: buddylist.actionbar=306:kept buddylist.group-header=800:bare buddylist.infobar=132:kept buddylist.menubar=836:bare chat.group-header=680:bare chat.input=327:kept chat.toolbar=326:kept community.category-header=800:bare community.channel-header=301:kept community.member-group-header=800:bare community.sidebar-header=142:kept statusbar=600:bare",
        "xfire-classic: buddylist.actionbar=100:kept buddylist.group-header=244:kept buddylist.infobar=124:kept buddylist.menubar=100:kept chat.group-header=200:kept chat.toolbar=100:kept community.category-header=306:kept community.channel-header=100:kept community.member-group-header=271:kept community.sidebar-header=124:kept statusbar=406:kept",
        "combat: buddylist.group-header=1049:bare buddylist.infobar=100:kept chat.group-header=102:kept chat.toolbar=107:kept community.category-header=200:kept community.channel-header=151:kept community.member-group-header=1049:bare community.sidebar-header=102:kept",
        "onyx: buddylist.actionbar=208:kept buddylist.group-header=350:kept buddylist.menubar=350:kept community.category-header=385:kept community.member-group-header=367:kept statusbar=123:kept",
        "spacecowboy: buddylist.group-header=1008:bare community.category-header=1008:bare community.member-group-header=1008:bare statusbar=123:kept",
        "stealthx: buddylist.group-header=742:bare buddylist.menubar=520:bare chat.group-header=501:bare community.category-header=742:bare community.member-group-header=742:bare",
        "total-chaos: buddylist.actionbar=100:kept buddylist.group-header=807:bare buddylist.menubar=100:kept community.category-header=807:bare community.member-group-header=807:bare",
        "xslimfire: buddylist.group-header=962:bare chat.group-header=773:bare community.category-header=962:bare community.member-group-header=962:bare statusbar=447:kept",
        "alloy: buddylist.group-header=642:bare buddylist.infobar=451:bare community.category-header=642:bare community.member-group-header=642:bare community.sidebar-header=504:bare statusbar=100:kept",
        "simple-x: buddylist.group-header=224:kept buddylist.infobar=100:kept community.category-header=271:kept community.member-group-header=244:kept community.sidebar-header=100:kept statusbar=1008:bare",
        "xglow: buddylist.group-header=1052:bare community.category-header=1052:bare community.member-group-header=1052:bare",
    ];
    assert_eq!(got, want);
}

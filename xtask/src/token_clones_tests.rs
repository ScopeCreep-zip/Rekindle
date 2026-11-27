//! The jscpd check's refusal of ignore markers and its report parsing.

use super::{
    marker_lines, parse_report, refused_suppressions, split_lists, strip_rust, Duplicate, Side,
};
use crate::structural::test_fixture_sources::{
    LINT_LIST, S5_DESKTOP_DEPS, S5_DESKTOP_SETUP, S5_HOST_DEPS, S5_HOST_SETUP, VARIANT_LIST,
};
use crate::structural::{LexedText, MIN_STRUCTURAL};

/// The seven cross-file clones jscpd 5.4.3 reported on the E4.1 S5 port at
/// the 2026-11-26 calibration (desktop line span, Host line span).
const S5_DEPS_CLONES: [(usize, usize, usize, usize); 3] = [
    (98, 107, 191, 200),
    (330, 336, 409, 415),
    (533, 545, 472, 481),
];
const S5_SETUP_CLONES: [(usize, usize, usize, usize); 4] = [
    (24, 29, 129, 134),
    (48, 61, 30, 42),
    (161, 184, 292, 311),
    (462, 473, 268, 279),
];

fn lines(src: &str, start: usize, end: usize) -> Vec<&str> {
    src.lines().skip(start - 1).take(end + 1 - start).collect()
}

/// The clones of `spans` that survive the calibration: stripping leaves
/// their lines as jscpd matched them, and both sides meet the floor.
fn surviving(a: &str, b: &str, spans: &[(usize, usize, usize, usize)]) -> usize {
    let (sa, sb) = (strip_rust(a), strip_rust(b));
    let (la, lb) = (LexedText::new(&sa), LexedText::new(&sb));
    spans
        .iter()
        .filter(|&&(a0, a1, b0, b1)| {
            let kept =
                lines(&sa, a0, a1) == lines(a, a0, a1) && lines(&sb, b0, b1) == lines(b, b0, b1);
            let floor = la.structural(a0, a1).min(lb.structural(b0, b1));
            println!("S5 clone {a0}-{a1} <> {b0}-{b1}: lines kept {kept}, structural {floor}");
            kept && floor >= MIN_STRUCTURAL
        })
        .count()
}

#[test]
fn the_s5_port_still_fails_jscpd_after_the_calibration() {
    let deps = surviving(S5_DESKTOP_DEPS, S5_HOST_DEPS, &S5_DEPS_CLONES);
    let setup = surviving(S5_DESKTOP_SETUP, S5_HOST_SETUP, &S5_SETUP_CLONES);
    println!("S5 jscpd clones surviving: deps {deps}/3, setup {setup}/4");
    assert!(deps >= 1 && setup >= 1);
}

#[test]
fn a_lint_list_and_a_variant_list_fall_under_the_floor() {
    let dir = std::env::temp_dir().join(format!("xtask-floor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("lib.rs"), strip_rust(LINT_LIST)).expect("write");
    std::fs::write(dir.join("all.rs"), strip_rust(VARIANT_LIST)).expect("write");
    let side = |name: &str, start, end| Side {
        name: name.to_owned(),
        start,
        end,
    };
    let clone = Duplicate {
        format: "rust".to_owned(),
        kind: Some("renamed".to_owned()),
        similarity: None,
        lines: 12,
        tokens: 50,
        first: side("lib.rs", 3, 16),
        second: side("all.rs", 6, 24),
    };
    let (kept, under) = split_lists(&dir, vec![clone]).expect("split");
    std::fs::remove_dir_all(&dir).expect("clean up");
    assert!(kept.is_empty() && under.len() == 1);
}

#[test]
fn an_ignore_marker_is_refused() {
    let marker = concat!("// jscpd", ":ignore-start");
    let src = format!("fn a() {{}}\n{marker}\nfn b() {{}}\n");
    assert_eq!(marker_lines(&src), vec![2]);
    let tsx = format!("const a = 1;\n/* {} */\n", concat!("jscpd", ":ignore-end"));
    assert_eq!(marker_lines(&tsx), vec![2]);
    assert!(marker_lines("fn plain() {}\n// copy detector\n").is_empty());
}

#[test]
fn a_marker_in_a_scanned_tree_and_a_config_file_are_refused() {
    let root = std::env::temp_dir().join(format!("xtask-token-clones-{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).expect("temp tree");
    std::fs::write(
        src.join("a.ts"),
        format!(
            "// {}\nexport const a = 1;\n",
            concat!("jscpd", ":ignore-start")
        ),
    )
    .expect("write");
    std::fs::write(root.join(".jscpd.json"), "{}").expect("write");
    let refused = refused_suppressions(&root).expect("scan");
    std::fs::remove_dir_all(&root).expect("clean up");
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(refused.iter().any(|r| r.starts_with("src/a.ts:1:")));
}

#[test]
fn the_report_parses() {
    let json = r#"{"duplicates":[{"format":"rust","kind":"renamed","lines":12,"tokens":56,
        "firstFile":{"name":"a.rs","start":108,"end":119},
        "secondFile":{"name":"b.rs","start":131,"end":142}}],"statistics":{}}"#;
    let dups = parse_report(json).expect("parse");
    assert_eq!(dups.len(), 1);
    assert_eq!(
        dups[0].to_string(),
        "rust renamed a.rs:108-119 <> b.rs:131-142 lines=12 tokens=56"
    );
}

/// rekindle-38's pair: bare signal declarations in two unrelated components.
const SIGNALS_A: &str = "const View = (props) => {\n  const [activeTag, setActiveTag] = createSignal<string | null>(null);\n  const [composerOpen, setComposerOpen] = createSignal(false);\n  const [title, setTitle] = createSignal(\"\");\n  const [body, setBody] = createSignal(\"\");\n  const [busy, setBusy] = createSignal(false);\n};\n";
const SIGNALS_B: &str = "const Popup = (props) => {\n  const [updatingRoleId, setUpdatingRoleId] = createSignal<string | null>(null);\n  const [editing, setEditing] = createSignal(false);\n  const [saving, setSaving] = createSignal(false);\n  const [error, setError] = createSignal(\"\");\n  const [open, setOpen] = createSignal(false);\n};\n";
/// `MemberProfilePopup.tsx:80-88`: five drafts seeded alike, twice in one file.
const DRAFTS: &str = "const Popup = (props) => {\n  const [bioDraft, setBioDraft] = createSignal(initialBio());\n  const [pronounsDraft, setPronounsDraft] = createSignal(initialPronouns());\n  const [customStatusDraft, setCustomStatusDraft] = createSignal(initialCustomStatus());\n  const [themeColorDraft, setThemeColorDraft] = createSignal(initialThemeColorHex());\n  const [badgesDraft, setBadgesDraft] = createSignal(initialBadges());\n  const [bioSaved, setBioSaved] = createSignal(initialBio());\n  const [pronounsSaved, setPronounsSaved] = createSignal(initialPronouns());\n  const [customStatusSaved, setCustomStatusSaved] = createSignal(initialCustomStatus());\n  const [themeColorSaved, setThemeColorSaved] = createSignal(initialThemeColorHex());\n  const [badgesSaved, setBadgesSaved] = createSignal(initialBadges());\n};\n";

fn ts_clone(a: (&str, usize, usize), b: (&str, usize, usize)) -> Duplicate {
    let side = |(name, start, end): (&str, usize, usize)| Side {
        name: name.to_owned(),
        start,
        end,
    };
    Duplicate {
        format: "tsx".to_owned(),
        kind: Some("renamed".to_owned()),
        similarity: None,
        lines: 5,
        tokens: 60,
        first: side(a),
        second: side(b),
    }
}

#[test]
fn declaration_runs_match_across_files_as_lists_but_stay_findings_in_one_file() {
    let dir = std::env::temp_dir().join(format!("xtask-ts-lists-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("View.tsx"), SIGNALS_A).expect("write");
    std::fs::write(dir.join("Popup.tsx"), SIGNALS_B).expect("write");
    std::fs::write(dir.join("Drafts.tsx"), DRAFTS).expect("write");
    let cross = ts_clone(("View.tsx", 2, 6), ("Popup.tsx", 2, 6));
    let same = ts_clone(("Drafts.tsx", 2, 6), ("Drafts.tsx", 7, 11));
    let with_header = ts_clone(("View.tsx", 1, 6), ("Popup.tsx", 1, 6));
    let (kept, lists) = split_lists(&dir, vec![cross, same, with_header]).expect("split");
    std::fs::remove_dir_all(&dir).expect("clean up");
    assert_eq!(lists.len(), 1, "the cross-file declaration run is a list");
    assert_eq!(lists[0].first.name, "View.tsx");
    assert_eq!(
        kept.len(),
        2,
        "the same-file run and a span with logic stay"
    );
    assert!(kept.iter().any(|d| d.first.name == "Drafts.tsx"));
}

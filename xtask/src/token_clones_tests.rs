//! The jscpd check's refusal of ignore markers and its report parsing.

use super::{marker_lines, parse_report, refused_suppressions};

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

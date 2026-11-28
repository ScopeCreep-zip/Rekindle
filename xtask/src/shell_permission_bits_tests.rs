use super::{has_bit_literal, scan};

#[test]
fn a_bigint_shift_literal_is_found() {
    assert!(has_bit_literal("export const SEND_MESSAGES = 1n << 16n;"));
    assert!(has_bit_literal("const X = 1n<<0n;"));
    assert!(has_bit_literal("  value: 1n << 45n,"));
}

#[test]
fn other_shifts_and_text_are_not() {
    assert!(!has_bit_literal("const mask = 1 << 3;"));
    assert!(!has_bit_literal("return current ^ bit;"));
    assert!(!has_bit_literal("// a << b in a comment"));
    assert!(!has_bit_literal("const s = `${a}n << ${b}`;"));
    assert!(!has_bit_literal("x <<"));
}

#[test]
fn scan_names_each_site() {
    let dir = std::env::temp_dir().join(format!("xtask-shell-bits-{}", std::process::id()));
    let src = dir.join("src").join("utils");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("permissions.ts"),
        "export const A = 1n << 2n;\nexport const ok = 3;\n",
    )
    .unwrap();
    std::fs::write(src.join("clean.ts"), "export const b = 1 << 2;\n").unwrap();
    let hits = scan(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        hits,
        vec!["src/utils/permissions.ts:1  export const A = 1n << 2n;".to_string()]
    );
}

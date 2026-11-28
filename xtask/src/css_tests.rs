use super::*;

#[test]
fn reads_nested_blocks_with_their_layers_and_lines() {
    let css = "@layer components {\n  /* a; b { */\n  .a, .b:hover {\n    color: var(--x, var(--y));\n    background: url(\"a;b\");\n  }\n  @media (prefers-contrast: more) {\n    :root { --z: #fff }\n  }\n}\n@keyframes k { 0% { color: red } }";
    let b = blocks(css);
    let rule = b.iter().find(|b| b.prelude == ".a, .b:hover").unwrap();
    assert_eq!(rule.selectors(), vec![".a", ".b:hover"]);
    assert!(rule.in_layer("components") && !rule.in_layer("sec"));
    assert_eq!(rule.decls.len(), 2);
    assert_eq!(rule.decls[0].line, 4);
    assert_eq!(var_refs(&rule.decls[0].value), vec!["--x", "--y"]);
    assert_eq!(rule.decls[1].value, "url(\"a;b\")");
    let root = b.iter().find(|b| b.prelude == ":root").unwrap();
    assert_eq!(root.decls[0].prop, "--z");
    assert!(root.in_layer("components"));
    let step = b.iter().find(|b| b.prelude == "0%").unwrap();
    assert!(step.in_keyframes());
    assert_eq!(rule.line, 3);
}

use super::*;

#[test]
fn duplicate_css_errors_include_each_file_and_line() {
    let root = std::env::temp_dir().join(format!("xtask-css-sites-{}", std::process::id()));
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("one.css"),
        "/* first site */\n.one { display: flex; flex-direction: column; gap: 6px; }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("two.css"),
        "\n\n.two { gap: 6px; display: flex; flex-direction: column; }\n",
    )
    .unwrap();

    let error = check_frontend_duplication(&root).unwrap_err().to_string();
    std::fs::remove_dir_all(root).unwrap();
    assert!(error.contains("identical CSS rule at 2 sites"), "{error}");
    assert!(error.contains("one.css:2"), "{error}");
    assert!(error.contains("two.css:3"), "{error}");
}

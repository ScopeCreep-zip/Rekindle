use super::sanitize_filename;

#[test]
fn strips_paths_and_traversal() {
    assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
    assert_eq!(sanitize_filename("a/b"), "b");
    assert_eq!(sanitize_filename("a\\b"), "b");
    assert_eq!(sanitize_filename(".."), "attachment");
    assert_eq!(sanitize_filename("report..pdf"), "report.pdf");
}

#[test]
fn strips_controls_and_spoofing() {
    assert_eq!(sanitize_filename("a\0b.txt"), "ab.txt");
    assert_eq!(sanitize_filename("evil\u{202E}gpj.exe"), "evilgpj.exe");
    assert_eq!(sanitize_filename("what?.txt"), "what.txt");
    assert_eq!(sanitize_filename("  -.bashrc"), "bashrc");
    assert_eq!(sanitize_filename(".bashrc"), "bashrc");
    assert_eq!(sanitize_filename("notes.txt. "), "notes.txt");
}

#[test]
fn guards_windows_device_names() {
    assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
    assert_eq!(sanitize_filename("lpt1"), "_lpt1");
    assert_eq!(sanitize_filename("COM10.txt"), "COM10.txt");
    assert_eq!(sanitize_filename("console.txt"), "console.txt");
}

#[test]
fn caps_length_keeping_the_extension() {
    let out = sanitize_filename(&format!("{}.pdf", "a".repeat(300)));
    assert_eq!(out.len(), 255);
    assert_eq!(
        std::path::Path::new(&out).extension(),
        Some(std::ffi::OsStr::new("pdf"))
    );
    let multibyte = sanitize_filename(&"é".repeat(300));
    assert!(multibyte.len() <= 255);
    assert!(multibyte.chars().all(|c| c == 'é'));
}

#[test]
fn empty_input_gets_a_name() {
    assert_eq!(sanitize_filename(""), "attachment");
    assert_eq!(sanitize_filename("\u{202E}\u{0}"), "attachment");
}

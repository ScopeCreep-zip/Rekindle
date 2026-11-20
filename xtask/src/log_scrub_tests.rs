use super::{binary_installs_hook, scan_subscribers};

#[test]
fn flags_a_subscriber_without_the_scrubber() {
    let bad = r#"
        fn init() {
            tracing_subscriber::fmt().with_writer(std::io::stderr).init();
        }
        fn layered() {
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer()).init();
        }
        fn global(s: S) {
            tracing::subscriber::set_global_default(s).expect("set");
        }
    "#;
    let hits = scan_subscribers("f.rs", bad);
    assert_eq!(hits.len(), 3, "{hits:?}");
    assert!(hits[0].starts_with("f.rs:3 "), "{hits:?}");
}

#[test]
fn allows_a_subscriber_through_the_scrubber() {
    let ok = r"
        fn init(writer: W) {
            tracing_subscriber::fmt()
                .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(writer))
                .with_ansi(false)
                .init();
        }
    ";
    assert!(scan_subscribers("f.rs", ok).is_empty());
}

#[test]
fn skips_test_code() {
    let test_only = r"
        #[cfg(test)]
        mod tests {
            fn quiet() {
                let _s = tracing::subscriber::set_default(tracing_subscriber::registry());
            }
        }
        #[tokio::test]
        async fn spans() {
            tracing_subscriber::fmt().init();
        }
    ";
    assert!(scan_subscribers("f.rs", test_only).is_empty());
}

#[test]
fn a_binary_installs_the_panic_hook() {
    let direct = "fn main() { rekindle_utils::log_scrub::install_panic_hook(); }";
    let with = "fn main() { log_scrub::install_panic_hook_with(|i| i.to_string()); }";
    let none = "fn main() { println!(\"hi\"); }";
    assert!(binary_installs_hook(direct, None));
    assert!(binary_installs_hook(with, None));
    assert!(!binary_installs_hook(none, None));
}

#[test]
fn a_binary_may_install_it_in_its_libs_run() {
    let main = "fn main() { rekindle_lib::run(); }";
    let lib = "pub fn run() { rekindle_utils::log_scrub::install_panic_hook(); }";
    let lib_without = "pub fn run() { start(); }";
    assert!(binary_installs_hook(main, Some(lib)));
    assert!(!binary_installs_hook(main, Some(lib_without)));
    assert!(!binary_installs_hook(main, None));
}

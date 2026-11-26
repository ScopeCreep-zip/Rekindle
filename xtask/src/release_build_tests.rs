use super::release_check_args;

#[test]
fn checks_every_member_in_release_but_the_driver() {
    let args = release_check_args();
    for wanted in ["--release", "--workspace"] {
        assert!(args.contains(&wanted), "missing {wanted}");
    }
    let excluded: Vec<_> = args
        .windows(2)
        .filter(|w| w[0] == "--exclude")
        .map(|w| w[1])
        .collect();
    assert_eq!(excluded, [crate::webdriver_release::CRATE]);
    assert!(
        !args.contains(&"--all-targets"),
        "tests name debug-only helpers"
    );
}

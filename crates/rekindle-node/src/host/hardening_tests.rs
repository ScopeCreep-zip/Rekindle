#[test]
fn core_dumps_are_forbidden_soft_and_hard() {
    super::forbid_core_dumps().expect("harden");
    let limit = rustix::process::getrlimit(rustix::process::Resource::Core);
    assert_eq!(limit.current, Some(0));
    assert_eq!(limit.maximum, Some(0));
    #[cfg(target_os = "linux")]
    assert_eq!(
        rustix::process::dumpable_behavior().expect("read"),
        rustix::process::DumpableBehavior::NotDumpable
    );
}

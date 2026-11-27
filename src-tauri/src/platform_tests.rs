use super::filter_nix_gui_stack;

#[test]
fn drops_nix_gui_stack_keeps_host_and_runtime_deps() {
    let input = concat!(
        "/nix/store/4i6mqkahmcjamn3gy5ni5r8hjidf7srh-webkitgtk-2.50.4+abi=4.1/lib:",
        "/nix/store/wy4c9khmxwp1vd2p6nbf1lpg0rpnk61v-glib-2.86.3/lib:",
        "/nix/store/dw1l57pjcr8ysf3r7xpx06mm8gg1xicd-gstreamer-1.26.5/lib:",
        "/nix/store/abc123-libsodium-1.0.20/lib:",
        "/nix/store/def456-libopus-1.5.2/lib:",
        "/usr/local/lib"
    );
    let filtered = filter_nix_gui_stack(input).expect("GUI entries must be dropped");
    assert_eq!(
        filtered,
        "/nix/store/abc123-libsodium-1.0.20/lib:/nix/store/def456-libopus-1.5.2/lib:/usr/local/lib",
        "REKINDLE_LIB_PATH deps and host paths pass through"
    );
}

#[test]
fn clean_path_is_left_untouched() {
    assert_eq!(filter_nix_gui_stack("/usr/lib:/opt/lib"), None);
    assert_eq!(
        filter_nix_gui_stack("/nix/store/abc-libsodium-1.0.20/lib"),
        None
    );
}

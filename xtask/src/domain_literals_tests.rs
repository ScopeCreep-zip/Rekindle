use super::is_label_shaped;

#[test]
fn label_shapes() {
    for s in [
        "rekindle-gov-op-v1",
        "rekindle-voice-receiver-report-v1",
        "rekindle-slot-",
        "rekindle-mek/1",
        "rekindle v1 vault-sqlcipher",
        "ReKindleRootKey",
    ] {
        assert!(is_label_shaped(s), "{s}");
    }
    for s in [
        "rekindle",
        "Rekindle",
        "rekindle-transport",
        "rekindle-protocol",
        "rekindle-e2e-{}",
        "rekindle_video::send",
        "rekindle://invite/{}/{}/{}",
        "rekindle/veilid",
        "rekindle.vault",
        "rekindle=info,warn",
        "rekindle node started and attached",
        "personal-sync-record",
    ] {
        assert!(!is_label_shaped(s), "{s}");
    }
}

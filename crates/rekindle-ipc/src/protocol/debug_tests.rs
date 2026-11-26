use super::*;

#[test]
fn no_passphrase_or_archive_reaches_debug_output() {
    let secret = "correct horse battery staple";
    for request in [
        IpcRequest::Unlock {
            identity: "pk".into(),
            passphrase: secret.into(),
        },
        IpcRequest::IdentityCreate {
            display_name: Some("Name".into()),
            passphrase: secret.into(),
        },
        IpcRequest::IdentityExport {
            passphrase: secret.into(),
        },
        IpcRequest::IdentityImport {
            archive: secret.into(),
            passphrase: secret.into(),
        },
        IpcRequest::IdentityRotate {
            passphrase: secret.into(),
        },
    ] {
        let shown = format!("{request:?}");
        assert!(!shown.contains(secret), "{shown}");
        assert!(shown.contains(REDACTED), "{shown}");
    }
}

/// Compiles only for a passphrase that is wiped when it drops.
fn wiped_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}

/// Roadmap 1.7: every request that carries a passphrase holds it in a
/// type that zeroizes on drop, so the Host's unlock path leaves no copy
/// of it in freed memory once the request is handled.
#[test]
fn every_passphrase_field_is_wiped_on_drop() {
    let check: fn(&IpcRequest) = |request| match request {
        IpcRequest::Unlock { passphrase, .. }
        | IpcRequest::IdentityCreate { passphrase, .. }
        | IpcRequest::IdentityExport { passphrase }
        | IpcRequest::IdentityImport { passphrase, .. }
        | IpcRequest::IdentityRotate { passphrase } => wiped_on_drop(passphrase),
        _ => {}
    };
    check(&IpcRequest::IdentityRotate {
        passphrase: "p".into(),
    });
}

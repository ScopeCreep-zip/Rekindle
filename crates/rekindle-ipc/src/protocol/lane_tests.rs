use super::*;

#[test]
fn identity_and_key_changes_are_exclusive() {
    for request in [
        IpcRequest::Lock,
        IpcRequest::Unlock {
            identity: String::new(),
            passphrase: rekindle_types::passphrase::Passphrase::default(),
        },
        IpcRequest::Logout,
        IpcRequest::Shutdown,
        IpcRequest::IdentityRotate {
            passphrase: rekindle_types::passphrase::Passphrase::default(),
        },
        IpcRequest::IdentityImport {
            archive: String::new(),
            passphrase: rekindle_types::passphrase::Passphrase::default(),
        },
        IpcRequest::IdentityWipe,
        IpcRequest::QuickWipeTrigger,
    ] {
        assert_eq!(request.lane(), Lane::Exclusive, "{}", request.name());
    }
    assert_eq!(IpcRequest::Status.lane(), Lane::Query);
    assert_eq!(IpcRequest::IdentityList.lane(), Lane::Query);
    assert_eq!(
        IpcRequest::DmTyping {
            peer_key: String::new(),
            typing: true
        }
        .lane(),
        Lane::Write
    );
}

/// Plan SK2-T1: the login window reports a device skin it could not
/// apply before any identity is unlocked, and no agent reports one.
#[test]
fn a_skin_apply_failure_is_open_and_the_users_own() {
    let failed = IpcRequest::SkinApplyFailed {
        window: "login".into(),
        skin: "builtin:xfire-classic".into(),
        cause: rekindle_types::dto::skin_library::SkinApplyFailure::SheetError {
            detail: String::new(),
        },
    };
    assert_eq!(failed.name(), "SkinApplyFailed");
    assert_eq!(failed.lane(), Lane::Write);
    assert_eq!(failed.required_clearance(), SecurityLevel::Open);
    assert_eq!(failed.agent_capability(), None);
}

#[test]
fn locked_state_requests_are_open_and_key_moves_are_admin() {
    assert_eq!(IpcRequest::Status.required_clearance(), SecurityLevel::Open);
    assert_eq!(
        IpcRequest::Unlock {
            identity: String::new(),
            passphrase: rekindle_types::passphrase::Passphrase::default(),
        }
        .required_clearance(),
        SecurityLevel::Open
    );
    assert_eq!(
        IpcRequest::IdentityExport {
            passphrase: rekindle_types::passphrase::Passphrase::default(),
        }
        .required_clearance(),
        SecurityLevel::Admin
    );
    assert_eq!(
        IpcRequest::PolicyReload.required_clearance(),
        SecurityLevel::Admin
    );
    assert_eq!(
        IpcRequest::FriendList.required_clearance(),
        SecurityLevel::Authenticated
    );
}

#[test]
fn name_omits_fields() {
    let request = IpcRequest::Unlock {
        identity: "pk".into(),
        passphrase: "secret".into(),
    };
    assert_eq!(request.name(), "Unlock");
}

use super::*;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn only_builtin_and_pkg_shapes_parse() {
    assert_eq!(
        parse("builtin:symbiosis"),
        Some(SkinId::Builtin("symbiosis"))
    );
    assert_eq!(parse(&format!("pkg:{HASH}")), Some(SkinId::Package(HASH)));
    for bad in [
        "builtin:",
        "builtin:Sym",
        "builtin:a/b",
        "pkg:",
        &format!("pkg:{}", HASH.to_uppercase()),
        &format!("pkg:{}", "0".repeat(63)),
        &format!("dev:{HASH}"),
        "symbiosis",
        "",
    ] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
}

#[test]
fn validate_request_refuses_bad_shapes() {
    let remove_builtin = IpcRequest::SkinRemove {
        id: "builtin:symbiosis".into(),
    };
    assert!(check(&remove_builtin).is_err());
    assert!(check(&IpcRequest::SkinImport { bytes: Vec::new() }).is_err());
    assert!(check(&IpcRequest::SkinSelect { id: "dev:x".into() }).is_err());
    assert!(check(&IpcRequest::SkinSetLogin {
        id: format!("dev:{HASH}")
    })
    .is_err());
    assert!(check(&IpcRequest::SkinSetLogin {
        id: "builtin:symbiosis".into()
    })
    .is_ok());
}

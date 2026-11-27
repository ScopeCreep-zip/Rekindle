use super::*;

fn served(cache: &str) -> Done<SkinServed> {
    Done {
        value: SkinServed {
            content_type: "text/css; charset=utf-8".into(),
            cache_control: cache.into(),
            bytes: b"@layer skin {}".to_vec(),
        },
        undo: None,
    }
}

fn header_of(r: &Response<Vec<u8>>, name: header::HeaderName) -> &str {
    r.headers().get(name).unwrap().to_str().unwrap()
}

#[test]
fn the_skin_line_names_only_the_files_basename() {
    assert_eq!(basename("/builtin:xfire/buddy-list.css"), "buddy-list.css");
    assert_eq!(basename("/pack"), "pack");
    assert_eq!(basename(""), "");
}

#[test]
fn the_hosts_headers_go_out_unchanged() {
    for cache in ["public, max-age=31536000, immutable", "no-store"] {
        let r = response(Ok(served(cache)));
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(header_of(&r, header::CACHE_CONTROL), cache);
        assert_eq!(
            header_of(&r, header::CONTENT_TYPE),
            "text/css; charset=utf-8"
        );
        assert_eq!(header_of(&r, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(r.body(), b"@layer skin {}");
    }
}

#[test]
fn an_unknown_address_or_a_bad_path_is_the_hosts_refusal() {
    // The Host answers 404 for an unknown package, an unknown image
    // and any path outside `/{package}/{file}` (`..` included).
    let r = response(Err(CommandError::Failed {
        code: 404,
        message: "no such skin file".into(),
        remediation: None,
    }));
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(header_of(&r, header::CACHE_CONTROL), "no-store");
    assert!(r.body().is_empty());
    let odd = response(Err(CommandError::Failed {
        code: 200,
        message: String::new(),
        remediation: None,
    }));
    assert_eq!(odd.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response(Err(CommandError::Cancelled)).status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

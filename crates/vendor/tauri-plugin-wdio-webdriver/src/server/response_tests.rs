use super::*;

#[test]
fn javascript_error_maps_stale_element_reference() {
    let err = WebDriverErrorResponse::javascript_error("stale element reference", None);
    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert_eq!(err.error, "stale element reference");
}

#[test]
fn javascript_error_preserves_generic_errors() {
    let err = WebDriverErrorResponse::javascript_error("TypeError: x is not a function", None);
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(err.error, "javascript error");
    assert_eq!(err.message, "TypeError: x is not a function");
}

#[test]
fn javascript_error_preserves_stacktrace_for_generic_errors() {
    let err = WebDriverErrorResponse::javascript_error(
        "ReferenceError: foo is not defined",
        Some("at eval:1:1".to_string()),
    );
    assert_eq!(err.error, "javascript error");
    assert_eq!(err.stacktrace, Some("at eval:1:1".to_string()));
}

#[test]
fn stale_element_reference_returns_not_found() {
    let err = WebDriverErrorResponse::stale_element_reference();
    assert_eq!(err.status, StatusCode::NOT_FOUND);
    assert_eq!(err.error, "stale element reference");
}

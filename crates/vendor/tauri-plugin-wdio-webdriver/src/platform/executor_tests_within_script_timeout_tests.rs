//! The script-timeout bound every execute path goes through (`within_script_timeout`).

use crate::platform::executor_util::within_script_timeout;
use crate::server::response::WebDriverErrorResponse;

#[tokio::test]
async fn an_answer_that_never_settles_is_a_script_timeout() {
    let result = within_script_timeout::<serde_json::Value>(20, std::future::pending()).await;
    let error = result.expect_err("a pending answer must time out");
    assert_eq!(error.error, "script timeout");
}

#[tokio::test]
async fn an_answer_within_the_bound_is_returned() {
    let result = within_script_timeout(1_000, async { Ok(serde_json::json!(7)) }).await;
    assert_eq!(result.expect("answered in time"), serde_json::json!(7));
}

#[tokio::test]
async fn an_error_within_the_bound_is_not_turned_into_a_timeout() {
    let result = within_script_timeout::<serde_json::Value>(1_000, async {
        Err(WebDriverErrorResponse::javascript_error("boom", None))
    })
    .await;
    assert_eq!(
        result.expect_err("the script's own error").error,
        "javascript error"
    );
}

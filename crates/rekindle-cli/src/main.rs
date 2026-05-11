#![recursion_limit = "512"]
//! Entrypoint for the `rekindle` CLI binary.
//!
//! Delegates to `v2::entrypoint::run()` — the complete rewrite against
//! the restructured crate architecture.

#![forbid(unsafe_code)]
#![deny(clippy::print_stdout)]

mod v2;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    v2::entrypoint::run().await;
}

//! Plan T2: the per-request debug line names the request's variant, never
//! its fields (threat model: no telemetry, no payload in logs).

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};

use super::Traced;

/// A writer that keeps what the subscriber formats.
#[derive(Clone, Default)]
struct Kept(Arc<Mutex<Vec<u8>>>);

impl Write for Kept {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The line `request` and `response` log, as formatted.
fn logged(request: &IpcRequest, response: &IpcResponse) -> String {
    let kept = Kept::default();
    let writer = kept.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        Traced::new(request, "buddy-list", Instant::now())
            .finish(response, Duration::from_millis(7));
    });
    let bytes = kept.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn the_line_names_the_command_never_its_fields() {
    let name = "Ann Quillfeather";
    let request = IpcRequest::ProfileSetNickname {
        display_name: name.into(),
    };
    let line = logged(
        &request,
        &IpcResponse::error(409, format!("{name} is taken")),
    );
    assert!(line.contains("command=\"ProfileSetNickname\""), "{line}");
    assert!(line.contains("window=\"buddy-list\""), "{line}");
    assert!(line.contains("took_ms=7"), "{line}");
    assert!(line.contains("outcome=\"error\" code=409"), "{line}");
    assert!(!line.contains("Quillfeather"), "the name leaked: {line}");
    assert!(!line.contains("taken"), "the answer leaked: {line}");
}

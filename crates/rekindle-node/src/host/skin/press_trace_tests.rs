use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::SkinAction;
use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun};

use super::{run_kind, Press};

fn press(action: SkinAction) -> Press {
    Press::of(&IpcRequest::RunSkinAction {
        window: "group-chat-1".into(),
        node: "CancelButton".into(),
        action,
    })
    .expect("a RunSkinAction is a press")
}

#[test]
fn a_mapped_press_names_its_run_and_never_its_ids() {
    let run = SkinRun::Shell(ShellOp::Call(CallOp::LeaveGroup {
        group_id: "VLD0:secretgroupidsecretgroupid".into(),
        call_id: "secretcallidsecretcallid".into(),
    }));
    let kind = run_kind(&run);
    let line = press(SkinAction::CancelVoiceChat).line(Some(&kind), &IpcResponse::ok(&run));
    assert_eq!(
        line,
        "skin action window=group-chat-1 node=CancelButton action=cancelVoiceChat \
             mapped=shell call leaveGroup outcome=ok"
    );
    assert!(!line.contains("secretgroupid"), "{line}");
    assert!(!line.contains("secretcallid"), "{line}");
}

#[test]
fn a_refused_press_names_the_refusal() {
    let refused = IpcResponse::error(
        409,
        "CancelVoiceChat acts on the window's call: none is held",
    );
    let line = press(SkinAction::CancelVoiceChat).line(None, &refused);
    assert_eq!(
        line,
        "skin action window=group-chat-1 node=CancelButton action=cancelVoiceChat mapped=- \
             outcome=refused 409: CancelVoiceChat acts on the window's call: none is held"
    );
}

#[test]
fn trace_logs_the_press_line_at_info() {
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sink = Sink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || writer.clone())
        .finish();
    let refused = IpcResponse::error(409, "no call is held");
    tracing::subscriber::with_default(subscriber, || {
        press(SkinAction::CancelVoiceChat).trace(None, &refused);
    });
    let logged = String::from_utf8(sink.0.lock().clone()).expect("utf-8 log");
    assert!(logged.contains("INFO"), "{logged}");
    assert!(
        logged.contains(
            "window=group-chat-1 node=CancelButton action=cancelVoiceChat mapped=- \
                 outcome=refused 409: no call is held"
        ),
        "{logged}"
    );
}

#[test]
fn a_request_that_is_no_press_has_no_line() {
    assert!(Press::of(&IpcRequest::Status).is_none());
}

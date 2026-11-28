//! One info line per `RunSkinAction` the Host answers (finding OBS1): a
//! real-app group-call run logged nothing a failed Cancel could be read
//! from, and the window's script cannot trace its own IPC. The line names
//! the window, the pressed node, the action, what the action mapped to
//! (a feature request by name, or the run's kinds, e.g. `shell call
//! leaveGroup`) and the answer (`ok`, `intent required`, or the refusal's
//! code and text). Only kind names are written, never an action's or a
//! run's arguments (ids, text); the log sink's scrub
//! (`rekindle_utils::log_scrub`) tags any id a refusal's text carries.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_run::SkinRun;

/// A press, read before its request resolves.
#[derive(Debug, Clone)]
pub(crate) struct Press {
    window: String,
    node: String,
    action: String,
}

impl Press {
    /// The press `request` carries, if it is a `RunSkinAction`.
    pub(crate) fn of(request: &IpcRequest) -> Option<Self> {
        let IpcRequest::RunSkinAction {
            window,
            node,
            action,
        } = request
        else {
            return None;
        };
        Some(Self {
            window: window.clone(),
            node: node.clone(),
            action: kinds(serde_json::to_value(action).as_ref().ok()),
        })
    }

    /// The press's one line: `mapped` is what it resolved to (`None` when
    /// it was refused before it mapped), `answer` the Host's answer.
    pub(crate) fn line(&self, mapped: Option<&str>, answer: &IpcResponse) -> String {
        let outcome = match answer {
            IpcResponse::Ok { .. } => "ok".to_owned(),
            IpcResponse::IntentRequired(_) => "intent required".to_owned(),
            IpcResponse::Error { code, message, .. } => format!("refused {code}: {message}"),
        };
        let node = if self.node.is_empty() {
            "-"
        } else {
            &self.node
        };
        format!(
            "skin action window={} node={node} action={} mapped={} outcome={outcome}",
            self.window,
            self.action,
            mapped.unwrap_or("-"),
        )
    }

    /// Log [`Self::line`] at info.
    pub(crate) fn trace(&self, mapped: Option<&str>, answer: &IpcResponse) {
        tracing::info!(target: "rekindle_node::skin", "{}", self.line(mapped, answer));
    }
}

/// What a feature action mapped to: the request's name.
pub(crate) fn request_kind(request: &IpcRequest) -> String {
    format!("request {}", request.name())
}

/// What a run is, by its kinds (`shell call leaveGroup`, `notBuiltYet`).
pub(crate) fn run_kind(run: &SkinRun) -> String {
    kinds(serde_json::to_value(run).as_ref().ok())
}

/// The `kind` chain of a `tag = "kind", content = "arg"` value: each kind,
/// then the arg's own kinds while the arg is itself tagged. An arg that is
/// data (an id, a text) is never written.
fn kinds(value: Option<&serde_json::Value>) -> String {
    let mut words = Vec::new();
    let mut at = value;
    while let Some(v) = at {
        if let Some(kind) = v.get("kind").and_then(serde_json::Value::as_str) {
            words.push(kind.to_owned());
            at = v.get("arg");
        } else {
            if let Some(unit) = v.as_str().filter(|_| words.is_empty()) {
                words.push(unit.to_owned());
            }
            at = None;
        }
    }
    if words.is_empty() {
        "-".to_owned()
    } else {
        words.join(" ")
    }
}

#[cfg(test)]
#[path = "press_trace_tests.rs"]
mod tests;

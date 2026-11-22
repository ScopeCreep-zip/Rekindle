//! `rekindle infoview` (plan E7.15, IV7): `InfoviewGet` for a subject, the
//! Host's sections printed in its order — headings, then their rows
//! (`rekindle_client::infoview`). JSON modes print the Host's answer as
//! sent. A subject the Host refuses prints its refusal, like every command.

use rekindle_client::infoview::{lines, InfoviewLine};
use rekindle_client::DaemonClient;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::infoview::{InfoviewView, CLI_WINDOW};

use crate::cli::InfoviewCmd;
use crate::output::{format, OutputMode};

/// The request `cmd` sends (no subcommand: our own page).
pub fn request(cmd: Option<&InfoviewCmd>) -> IpcRequest {
    IpcRequest::InfoviewGet {
        subject: InfoviewCmd::subject(cmd),
        window: CLI_WINDOW.to_owned(),
    }
}

/// Send `cmd`'s request and print the Host's view.
pub async fn dispatch(
    cmd: Option<&InfoviewCmd>,
    client: &DaemonClient,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let value = crate::helpers::send(client, request(cmd)).await?;
    if mode.is_structured() {
        return format::print_structured(&value, mode);
    }
    let view: InfoviewView = serde_json::from_value(value)?;
    format::print_text(&text(&view))
}

/// The view as text: each heading on its own line, its rows indented.
fn text(view: &InfoviewView) -> String {
    lines(view)
        .into_iter()
        .map(|line| match line {
            InfoviewLine::Heading(h) => h,
            InfoviewLine::Row(r) => format!("  {r}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

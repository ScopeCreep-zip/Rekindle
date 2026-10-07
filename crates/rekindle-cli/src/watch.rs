//! Streaming commands: subscribe, print each matching event, stop on
//! Ctrl-C (`channel watch`, `dm watch`, `voice join --watch`).
//!
//! Text mode prints one line per event; `--format json|jsonl` and
//! `--script` print each event as one JSON line, for pipelines.

use rekindle_client::DaemonClient;
use rekindle_types::subscription_events::{SubscriptionEvent, SubscriptionFilter};

use crate::output::{format, OutputMode};

/// Subscribe with `filters` and print every event `render` accepts until
/// Ctrl-C. `render` returns the text line for an event it wants shown.
///
/// # Errors
/// The subscription is refused, or the daemon connection closes.
pub async fn stream(
    client: &DaemonClient,
    filters: Vec<SubscriptionFilter>,
    mode: OutputMode,
    mut render: impl FnMut(&SubscriptionEvent) -> Option<String>,
) -> anyhow::Result<()> {
    let mut events = client
        .take_event_receiver()
        .ok_or_else(|| anyhow::anyhow!("this connection's event stream is already in use"))?;
    client.subscribe(filters).await?;
    loop {
        tokio::select! {
            interrupt = tokio::signal::ctrl_c() => {
                interrupt?;
                return Ok(());
            }
            event = events.recv() => {
                let Some(event) = event else {
                    anyhow::bail!("the daemon closed the connection");
                };
                let Some(line) = render(&event) else { continue };
                if mode.is_structured() {
                    format::print_jsonl(&event)?;
                } else {
                    format::print_text(&line)?;
                }
            }
        }
    }
}

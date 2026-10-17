//! Streaming commands: subscribe, print each matching event, stop on
//! Ctrl-C (`channel watch`, `dm watch`, `voice join --watch`).
//!
//! Text mode prints one line per event; `--format json|jsonl` and
//! `--script` print each event as one JSON line, for pipelines.

use rekindle_client::DaemonClient;
use rekindle_types::subscription_events::{StreamFrame, SubscriptionEvent, SubscriptionFilter};

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
    client.subscribe(filters, None).await?;
    loop {
        tokio::select! {
            interrupt = tokio::signal::ctrl_c() => {
                interrupt?;
                return Ok(());
            }
            frame = events.recv() => {
                let event = match frame {
                    None => anyhow::bail!("the daemon closed the connection"),
                    Some(StreamFrame::Event(sequenced)) => sequenced.event,
                    // A watch keeps no state to reload (plan D3.2).
                    Some(StreamFrame::InvalidSession) => anyhow::bail!(
                        "the event stream was reset (the identity was locked, or this watch \
                         fell behind); run it again"
                    ),
                    Some(StreamFrame::Resumed(_)) => continue,
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

//! Terminal lifecycle wrapper.
//!
//! Wraps [`ratatui::DefaultTerminal`] with an async event task that
//! multiplexes terminal events, tick/render timers, and transport
//! notifications into a single [`Event`] channel.

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEventKind};
use futures_util::{FutureExt, StreamExt};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::event::Event;
use crate::config::schema::TuiConfig;

/// Terminal wrapper owning the ratatui terminal and the event loop task.
///
/// The event loop runs in a spawned tokio task, producing [`Event`] values
/// into an unbounded channel. The App main loop consumes from `event_rx`.
///
/// On `Drop`, the terminal is restored via `ratatui::restore()`.
pub struct Tui {
    pub terminal: ratatui::DefaultTerminal,
    #[allow(dead_code)] // Held to keep the spawned event task alive until Drop.
    task: JoinHandle<()>,
    pub cancellation_token: CancellationToken,
    pub event_rx: UnboundedReceiver<Event>,
}

impl Tui {
    /// Create a new Tui with terminal initialization and event task.
    pub fn new(config: &TuiConfig) -> color_eyre::Result<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let cancellation_token = CancellationToken::new();

        let terminal = ratatui::init();

        if config.mouse {
            crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture)?;
        }

        crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;

        let task = Self::spawn_event_task(
            event_tx.clone(),
            cancellation_token.clone(),
            config.tick_rate,
            config.frame_rate,
        );

        Ok(Self {
            terminal,
            task,
            cancellation_token,
            event_rx,
        })
    }

    /// Spawn the event loop task.
    ///
    /// Multiplexes four event sources via `tokio::select!`:
    /// 1. Cancellation token
    /// 2. Tick interval
    /// 3. Render interval
    /// 4. Crossterm terminal events (Press-only filter)
    ///
    /// Subscription events from the daemon are consumed directly by App::run()
    /// via the dedicated event_rx channel — they do not flow through this task.
    fn spawn_event_task(
        tx: UnboundedSender<Event>,
        token: CancellationToken,
        tick_rate: f64,
        frame_rate: f64,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(Duration::from_secs_f64(1.0 / tick_rate));
            let mut render_interval =
                tokio::time::interval(Duration::from_secs_f64(1.0 / frame_rate));
            let mut reader = EventStream::new();

            let _ = tx.send(Event::Init);

            loop {
                let event = tokio::select! {
                    () = token.cancelled() => break,
                    _ = tick_interval.tick() => Event::Tick,
                    _ = render_interval.tick() => Event::Render,
                    Some(Ok(evt)) = reader.next().fuse() => {
                        match evt {
                            CrosstermEvent::Key(k) if k.kind == KeyEventKind::Press => {
                                Event::Key(k)
                            }
                            CrosstermEvent::Mouse(m) => Event::Mouse(m),
                            CrosstermEvent::Resize(w, h) => Event::Resize(w, h),
                            CrosstermEvent::FocusGained => Event::FocusGained,
                            CrosstermEvent::FocusLost => Event::FocusLost,
                            CrosstermEvent::Paste(s) => Event::Paste(s),
                            CrosstermEvent::Key(_) => continue,
                        }
                    }
                };

                if tx.send(event).is_err() {
                    break;
                }
            }
        })
    }

    /// Signal the event task to shut down.
    pub fn stop(&self) {
        self.cancellation_token.cancel();
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::DisableMouseCapture,
            crossterm::event::DisableBracketedPaste,
        );
        ratatui::restore();
        self.stop();
    }
}

impl Deref for Tui {
    type Target = ratatui::DefaultTerminal;
    fn deref(&self) -> &Self::Target {
        &self.terminal
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.terminal
    }
}

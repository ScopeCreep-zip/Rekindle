//! Terminal lifecycle wrapper.
//!
//! Wraps ratatui's DefaultTerminal with an async event task that
//! multiplexes terminal events, tick/render timers into a single
//! Event channel. Multiple TUI instances can run in separate tmux
//! panes simultaneously — each owns its own terminal handle and
//! event loop.

use std::ops::{Deref, DerefMut};
use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEventKind};
use futures_util::{FutureExt, StreamExt};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::event::Event;
use crate::v2::config::schema::TuiConfig;

/// Terminal wrapper owning the ratatui terminal and the event loop task.
pub struct Tui {
    pub terminal: ratatui::DefaultTerminal,
    #[allow(dead_code)]
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
            crossterm::execute!(
                std::io::stdout(),
                crossterm::event::EnableMouseCapture
            )?;
        }

        crossterm::execute!(
            std::io::stdout(),
            crossterm::event::EnableBracketedPaste
        )?;

        let task = Self::spawn_event_task(
            event_tx,
            cancellation_token.clone(),
            config.tick_rate,
            config.frame_rate,
        );

        Ok(Self { terminal, task, cancellation_token, event_rx })
    }

    /// Spawn the event loop task — multiplexes 4 sources via tokio::select!
    fn spawn_event_task(
        tx: UnboundedSender<Event>,
        token: CancellationToken,
        tick_rate: f64,
        frame_rate: f64,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick_interval =
                tokio::time::interval(Duration::from_secs_f64(1.0 / tick_rate));
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
                            CrosstermEvent::Key(k) if k.kind == KeyEventKind::Press => Event::Key(k),
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
    fn deref(&self) -> &Self::Target { &self.terminal }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.terminal }
}

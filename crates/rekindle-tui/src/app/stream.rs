//! The TUI's place in the host's event stream (plan D3.2).
//!
//! The TUI keeps the cursor of the last frame it processed, in memory and
//! in the owner-only runtime directory, so a TUI killed mid-burst and
//! started again resumes from where it stopped: the host replays what
//! followed the cursor, then `Resumed`. When the host cannot serve it
//! (`InvalidSession`: another unlock, evicted, or the TUI fell behind) the
//! TUI reloads its data and subscribes anew from the live stream.

use rekindle_types::subscription_events::{Cursor, StreamFrame};

use super::App;
use crate::action::Action;

/// Where the cursor is kept between runs.
fn cursor_path() -> Option<std::path::PathBuf> {
    rekindle_ipc::runtime_dir()
        .ok()
        .map(|dir| dir.join("tui.cursor"))
}

/// The cursor the last run kept, if any.
pub(crate) fn load_cursor() -> Option<Cursor> {
    let bytes = std::fs::read(cursor_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_cursor(cursor: Option<Cursor>) {
    let Some(path) = cursor_path() else {
        return;
    };
    let written = match cursor {
        Some(cursor) => serde_json::to_vec(&cursor)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&path, bytes)),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = written {
        tracing::debug!(error = %e, "TUI cursor not kept");
    }
}

impl App {
    /// Handle one frame: an event becomes an action; the cursor follows
    /// every frame; `InvalidSession` reloads and subscribes again.
    pub(crate) async fn handle_frame(&mut self, frame: StreamFrame) {
        match frame {
            StreamFrame::Event(sequenced) => {
                self.cursor = Some(sequenced.cursor);
                if let Some(action) = self.subscription_event_to_action(sequenced.event) {
                    let _ = self.action_tx.send(action);
                }
            }
            StreamFrame::Resumed(cursor) => self.cursor = Some(cursor),
            StreamFrame::InvalidSession => {
                self.cursor = None;
                self.load_dashboard_data();
                if let Err(e) = self.client.subscribe_all(None).await {
                    let _ = self.action_tx.send(Action::ShowToast {
                        message: format!("event stream lost: {e}"),
                        level: crate::action::ToastLevel::Error,
                    });
                }
            }
        }
    }

    /// Keep the cursor for the next run once it has moved.
    pub(crate) fn keep_cursor(&mut self) {
        if self.cursor != self.kept_cursor {
            save_cursor(self.cursor);
            self.kept_cursor = self.cursor;
        }
    }
}

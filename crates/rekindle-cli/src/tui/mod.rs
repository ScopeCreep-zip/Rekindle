//! TUI system — interactive terminal user interface.
//!
//! This module is feature-gated behind `tui`. It provides the full
//! interactive TUI with dashboard, channel watch, DM inbox, voice
//! session, friend list, doctor, and community info views.
//!
//! The TUI system is implemented in M2. This module provides the
//! entry point that M1's `main.rs` references when the `tui` feature
//! is enabled.

use crate::cli::Cli;

/// TUI entry point — launches the interactive terminal interface.
///
/// Called from `main.rs` when the output mode is `Tui`. Sets up the
/// terminal, creates the App with transport bridge, runs the event
/// loop, and restores the terminal on exit.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    let config = crate::config::load(cli.config.as_deref())?;
    crate::config::validate(&config)?;

    // Load session
    let session_path = crate::helpers::session_path()?;
    let session = rekindle_transport::Session::load(&session_path)?
        .ok_or_else(|| {
            crate::error::CliError::NotInitialized(
                "no identity found — run: rekindle init".into(),
            )
        })?;

    // Acquire transport
    let handle = crate::transport::acquire(&config, &cli).await?;

    // Initialize terminal with color-eyre panic hook (already installed in main.rs)
    let mut terminal = ratatui::init();

    // Run the TUI event loop
    // M2 will replace this with the full App::run() implementation.
    // For M1, we render a status screen that proves the TUI pipeline works.
    let result = run_status_screen(&mut terminal, &handle, &session);

    // Restore terminal (also happens in Drop, but explicit is clearer)
    ratatui::restore();

    // Shutdown transport
    handle.shutdown_if_owned().await?;

    result
}

/// Minimal TUI screen showing node status.
///
/// This is the M1 proof-of-concept TUI that verifies the ratatui
/// pipeline compiles and renders. M2 replaces it with the full
/// App struct, event loop, component system, and view registry.
fn run_status_screen(
    terminal: &mut ratatui::DefaultTerminal,
    handle: &crate::transport::TransportHandle,
    session: &rekindle_transport::Session,
) -> anyhow::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::{Style, Stylize};
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Paragraph};

    loop {
        let snapshot = handle.node().status_snapshot();
        let peer_summary = handle.node().peers().read().circuit_summary();

        terminal.draw(|frame| {
            let area = frame.area();

            let [header, content, footer] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Fill(1),
                Constraint::Length(1),
            ])
            .areas(area);

            // Header
            let header_line = Line::from(vec![
                " rekindle ".bold(),
                format!(
                    " {} ",
                    if snapshot.is_attached {
                        "[ONLINE]"
                    } else {
                        "[OFFLINE]"
                    }
                )
                .into(),
                format!(" {} peers ", peer_summary.total).into(),
            ]);
            frame.render_widget(Paragraph::new(header_line), header);

            // Content — node status + identity
            let status_text = vec![
                Line::from(""),
                Line::from(format!(
                    "  Attachment:     {} {}",
                    if snapshot.is_attached { "[OK]" } else { "[--]" },
                    snapshot.attachment
                )),
                Line::from(format!(
                    "  Public Internet: {}",
                    if snapshot.public_internet_ready {
                        "[OK] ready"
                    } else {
                        "[--] not ready"
                    }
                )),
                Line::from(format!(
                    "  Uptime:         {}",
                    crate::helpers::format_uptime(snapshot.uptime_secs)
                )),
                Line::from(format!(
                    "  Peers:          {} healthy, {} degraded, {} circuit open",
                    peer_summary.healthy, peer_summary.degraded, peer_summary.circuit_open
                )),
                Line::from(format!(
                    "  Route:          {}",
                    if snapshot.route_allocated {
                        "[OK] allocated"
                    } else {
                        "[--] none"
                    }
                )),
                Line::from(""),
                Line::from(format!(
                    "  Identity:       {}",
                    crate::helpers::abbreviate_key(&session.identity.public_key_hex)
                )),
                Line::from(format!(
                    "  Display name:   {}",
                    session.identity.display_name
                )),
                Line::from(format!(
                    "  Communities:    {}",
                    session.communities.len()
                )),
                Line::from(""),
                Line::from("  Full TUI dashboard coming in M2."),
            ];

            let block = Block::bordered().title(" Node Status ");
            let para = Paragraph::new(status_text).block(block);
            frame.render_widget(para, content);

            // Footer
            let footer_line =
                Line::from(" q quit | ? help | Tab navigate ").style(Style::new().dim());
            frame.render_widget(Paragraph::new(footer_line), footer);
        })?;

        // Poll for keyboard events with 250ms timeout (allows periodic refresh)
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

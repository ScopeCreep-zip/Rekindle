//! Status bar component — bottom line of the TUI.
//!
//! Displays three sections:
//! - Left: mode badge `[NORMAL]` or `[INSERT]` with distinct styling
//! - Center: breadcrumb path (Community / #channel)
//! - Right: node status indicator with glyph + text + color
//!
//! The mode badge uses BOTH a text label AND a distinct background color
//! so the user always knows their mode, even without color.
//!
//! Source patterns:
//! - siggy `ui/status_bar.rs` — `[NORMAL]`/`[INSERT]` mode indicator
//! - vortix `ui/widgets/footer.rs` — two-tier hint system

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use rekindle_types::subscription_events::{
    NetworkEvent, OwnRouteClass, OwnRouteDownSet, OwnRouteOutage, RoutePauseCause,
};

use crate::theme::ThemeManager;

/// The route warnings the status line shows (plan RV1, threat model L2):
/// the Host's `RoutePaused` and `RouteDeathsAboveBaseline`, folded in as
/// they arrive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteAlert {
    /// Why a route class is down with no new route, if one is.
    paused: Option<RoutePauseCause>,
    /// The latest death warning: the class, its deaths within the window,
    /// how many were young, and the window in seconds.
    deaths: Option<(OwnRouteClass, u32, u32, u64)>,
    /// Which own route classes are down, and since when (plan RV1g).
    down: OwnRouteDownSet,
}

/// "since HH:MM" of an epoch-ms instant in local clock time, with the
/// date when it is not today; an instant chrono cannot represent says
/// only how long it has been retrying, never a made-up time.
fn since_text(since_ms: u64) -> String {
    match local_clock(since_ms, chrono::Local::now()) {
        Some(at) => format!(" since {at}"),
        None => String::new(),
    }
}

/// `since_ms` as local "HH:MM", or "YYYY-MM-DD HH:MM" when its local day is
/// not `now`'s.
fn local_clock(since_ms: u64, now: chrono::DateTime<chrono::Local>) -> Option<String> {
    let millis = i64::try_from(since_ms).ok()?;
    let at = chrono::DateTime::from_timestamp_millis(millis)?.with_timezone(&chrono::Local);
    let format = if at.date_naive() == now.date_naive() {
        "%H:%M"
    } else {
        "%Y-%m-%d %H:%M"
    };
    Some(at.format(format).to_string())
}

impl RouteAlert {
    /// Fold a network event in; any other event leaves it as it is.
    pub fn fold(&mut self, event: &NetworkEvent) {
        match event {
            NetworkEvent::RoutePaused { cause } => self.paused = Some(*cause),
            NetworkEvent::RouteResumed {} => self.paused = None,
            NetworkEvent::RouteDeathsAboveBaseline {
                class,
                deaths,
                young,
                window_secs,
                ..
            } => self.deaths = Some((*class, *deaths, *young, *window_secs)),
            NetworkEvent::OwnRouteDown {
                class,
                since_ms,
                reason,
            } => {
                *self.down.entry(*class) = Some(OwnRouteOutage {
                    since_ms: *since_ms,
                    reason: reason.clone(),
                });
            }
            NetworkEvent::OwnRouteUp { class } => *self.down.entry(*class) = None,
            _ => {}
        }
    }

    /// The down set a status snapshot carries (a TUI started mid-outage).
    pub fn load_down(&mut self, down: &OwnRouteDownSet) {
        self.down = down.clone();
    }

    /// The outage text: "Unreachable — retrying since HH:MM" (local) for the
    /// general class, "Calls unreachable — …" for the media class.
    fn down_text(&self) -> Option<String> {
        let general = self
            .down
            .general
            .as_ref()
            .map(|o| format!("Unreachable — retrying{}", since_text(o.since_ms)));
        let media = self
            .down
            .media
            .as_ref()
            .map(|o| format!("Calls unreachable — retrying{}", since_text(o.since_ms)));
        match (general, media) {
            (Some(g), Some(m)) => Some(format!("{g} · {m}")),
            (g, m) => g.or(m),
        }
    }

    /// The status line's warning text, if there is one.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        let paused = self.paused.map(|cause| match cause {
            RoutePauseCause::Exhausted => {
                "inbound route paused after repeated failures: possible attack".to_string()
            }
            RoutePauseCause::Unreadable => {
                "inbound route paused: route budget unreadable".to_string()
            }
        });
        let deaths = self.deaths.map(|(class, deaths, young, window)| {
            let class = match class {
                OwnRouteClass::General => "general",
                OwnRouteClass::Media => "media",
            };
            format!(
                "{class} route deaths above baseline: {deaths} in {}m ({young} young)",
                window / 60
            )
        });
        let parts: Vec<String> = [self.down_text(), paused, deaths]
            .into_iter()
            .flatten()
            .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// Current input mode for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
}

/// Data needed to render the status bar.
pub struct StatusBarState {
    /// Current input mode.
    pub mode: Mode,
    /// Breadcrumb path — e.g., "dev-team / #general".
    pub breadcrumb: String,
    /// Whether the node is attached to the network.
    pub node_attached: bool,
    /// Veilid's attachment strength as the Host smoothed it, 0-5
    /// (`NetworkEvent::Strength`, plan E7.2): the GUI's bars.
    pub bars: u8,
    /// Keybinding hint string.
    pub hints: String,
    /// The route warning ([`RouteAlert::text`]), if any.
    pub route_alert: Option<String>,
}

/// Render the status bar into a single-line area.
pub fn render(frame: &mut Frame, area: Rect, state: &StatusBarState, theme: &ThemeManager) {
    let mode_badge = match state.mode {
        Mode::Normal => Span::styled(
            " [NORMAL] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("accent.primary"))
                .bold(),
        ),
        Mode::Insert => Span::styled(
            " [INSERT] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("success"))
                .bold(),
        ),
        Mode::Search => Span::styled(
            " [SEARCH] ",
            Style::default()
                .fg(theme.color("bg.base"))
                .bg(theme.color("warning"))
                .bold(),
        ),
    };

    let breadcrumb = if state.breadcrumb.is_empty() {
        Span::raw("")
    } else {
        Span::styled(format!(" {} ", state.breadcrumb), Style::new().dim())
    };

    let node_glyph = theme.status_glyph(state.node_attached);
    let node_text = if state.node_attached {
        format!(
            " {node_glyph} {} ",
            bars_text(state.bars, theme.use_unicode())
        )
    } else {
        format!(" {node_glyph} offline ")
    };
    let node_span = Span::styled(node_text, Style::new().dim());

    let separator = Span::raw(" │ ");

    let hints = Span::styled(format!(" {} ", state.hints), Style::new().dim());

    let mut spans = vec![mode_badge, breadcrumb, separator.clone(), node_span];
    if let Some(alert) = &state.route_alert {
        spans.push(Span::styled(
            format!(" {alert} "),
            Style::default().fg(theme.color("warning")).bold(),
        ));
    }
    spans.push(separator);
    spans.push(hints);
    let line = Line::from(spans);

    frame.render_widget(Paragraph::new(line), area);
}

/// The indicator's five bars, rising, lit up to `bars`.
fn bars_text(bars: u8, unicode: bool) -> String {
    const LIT: [char; 5] = ['▁', '▂', '▄', '▆', '█'];
    (1..=5u8)
        .map(|level| match (bars >= level, unicode) {
            (true, true) => LIT[usize::from(level - 1)],
            (false, true) => '·',
            (true, false) => '|',
            (false, false) => '.',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{bars_text, local_clock, RouteAlert};
    use rekindle_types::subscription_events::{
        NetworkEvent, OwnRouteClass, OwnRouteDownSet, OwnRouteOutage, RoutePauseCause,
    };

    /// Plan RV1: the pause, with its cause, and the deaths warning, with its
    /// young deaths, are folded into the status line; a refill clears the
    /// pause.
    #[test]
    fn the_status_line_shows_the_route_pause_and_the_deaths_warning() {
        let mut alert = RouteAlert::default();
        assert_eq!(alert.text(), None);
        alert.fold(&NetworkEvent::RoutePaused {
            cause: RoutePauseCause::Unreadable,
        });
        assert_eq!(
            alert.text().as_deref(),
            Some("inbound route paused: route budget unreadable")
        );
        alert.fold(&NetworkEvent::RoutePaused {
            cause: RoutePauseCause::Exhausted,
        });
        alert.fold(&NetworkEvent::RouteDeathsAboveBaseline {
            class: OwnRouteClass::General,
            deaths: 5,
            young: 2,
            window_secs: 3600,
            baseline: 3,
        });
        assert_eq!(
            alert.text().as_deref(),
            Some(
                "inbound route paused after repeated failures: possible attack · \
                 general route deaths above baseline: 5 in 60m (2 young)"
            )
        );
        alert.fold(&NetworkEvent::RouteResumed {});
        assert_eq!(
            alert.text().as_deref(),
            Some("general route deaths above baseline: 5 in 60m (2 young)")
        );
        // Plan RV1g: an outage shows from the event and from a snapshot,
        // and clears on the way up.
        // The times are local clock time: the expected text is computed
        // with the same local conversion, so the test holds in any zone.
        alert = RouteAlert::default();
        let now = chrono::Local::now();
        let today_ms = u64::try_from(now.timestamp_millis()).expect("after 1970");
        alert.fold(&NetworkEvent::OwnRouteDown {
            class: OwnRouteClass::General,
            since_ms: today_ms,
            reason: "allocated route failed to test".into(),
        });
        assert_eq!(
            alert.text(),
            Some(format!(
                "Unreachable — retrying since {}",
                now.format("%H:%M")
            ))
        );
        alert.fold(&NetworkEvent::OwnRouteUp {
            class: OwnRouteClass::General,
        });
        assert_eq!(alert.text(), None);
        alert.load_down(&OwnRouteDownSet {
            general: None,
            media: Some(OwnRouteOutage {
                since_ms: 9 * 60_000,
                reason: "r".into(),
            }),
        });
        let then = chrono::DateTime::from_timestamp_millis(9 * 60_000)
            .expect("in range")
            .with_timezone(&chrono::Local);
        assert_eq!(
            alert.text(),
            Some(format!(
                "Calls unreachable — retrying since {}",
                then.format("%Y-%m-%d %H:%M")
            ))
        );
        assert_eq!(local_clock(u64::MAX, now), None);
    }

    /// The TUI shows the Host's bars as the GUI does (plan E7.2).
    #[test]
    fn the_bars_light_up_to_the_hosts_strength() {
        assert_eq!(bars_text(3, true), "▁▂▄··");
        assert_eq!(bars_text(0, false), ".....");
        assert_eq!(bars_text(5, false), "|||||");
    }
}

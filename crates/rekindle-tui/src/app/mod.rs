//! TUI application — TEA main loop and state owner.
//!
//! `App` owns the view registry, notification stack, keybinding store,
//! theme manager, overlay state, and daemon client. Runs the main event
//! loop: drain events → convert to actions → process → render.
//!
//! # Module layout
//!
//! - `mod.rs`      — App struct, new(), run()
//! - `events.rs`   — Event→Action translation, transport notification handling
//! - `reducer.rs`  — process_action() — the TEA reducer
//! - `render.rs`   — draw(), overlays, breadcrumb, search items, tab transitions
//! - `commands.rs` — Async IPC command spawning (all daemon requests)
//! - `game_commands.rs` — installed games and `GameLaunch`

mod activity_commands;
mod commands;
mod events;
mod friend_commands;
mod game_commands;
mod group_dm_commands;
mod profile_commands;
mod reducer;
mod render;
mod skin;
mod stream;

pub(crate) use stream::load_cursor;

/// Centre a rect inside another — shared with the overlay components,
/// which each carried a byte-identical copy.
pub(crate) use render::centered_rect;

use std::sync::Arc;

use tokio::sync::mpsc;

use super::action::Action;
use super::components::intent_overlay::IntentOverlay;
use super::components::search_overlay::SearchOverlay;
use super::components::spinner::Spinner;
use super::components::tab_bar::Tab;
use super::components::toast::NotificationStack;
use super::keybinds::KeymapStore;
use super::navigator::Navigator;
use super::terminal::Tui;
use super::theme::ThemeManager;
use rekindle_client::DaemonClient;

/// TUI application state and main loop.
///
/// Navigation state (views, tabs, focus, input mode, overlays) is owned
/// by [`Navigator`]. App owns business state (daemon client, cached data,
/// notifications, clipboard) and the main event loop.
pub struct App {
    pub(crate) should_quit: bool,

    pub(crate) action_tx: mpsc::UnboundedSender<Action>,
    pub(crate) action_rx: mpsc::UnboundedReceiver<Action>,

    pub(crate) client: Arc<DaemonClient>,

    pub(crate) theme: ThemeManager,
    pub(crate) keymap: KeymapStore,

    /// Unified navigation state — owns views, tabs, focus, overlays.
    pub(crate) nav: Navigator,

    pub(crate) notifications: NotificationStack,
    pub(crate) search: SearchOverlay,
    pub(crate) intent: IntentOverlay,
    pub(crate) loading_spinner: Spinner,
    pub(crate) node_was_connected: bool,
    /// The Host's smoothed network bars (`NetworkEvent::Strength`, plan E7.2).
    pub(crate) network_bars: u8,
    pub(crate) clipboard: Option<arboard::Clipboard>,
    pub(crate) clipboard_clear_at: Option<std::time::Instant>,
    pub(crate) idle_frames: u32,

    /// Cached community data from the last CommunityList response.
    /// Used for breadcrumbs and tab labels without IPC round-trips.
    pub(crate) cached_communities: Vec<CachedCommunity>,
    /// Cached identity from the last Status/IdentityShow response.
    pub(crate) cached_identity: Option<CachedIdentity>,
    /// The last stream frame's cursor (`stream`), and the one last kept on
    /// disk.
    pub(crate) cursor: Option<rekindle_types::subscription_events::Cursor>,
    pub(crate) kept_cursor: Option<rekindle_types::subscription_events::Cursor>,
}

/// Minimal community cache for UI labels and navigation.
#[derive(Clone)]
pub(crate) struct CachedCommunity {
    pub anchor: String,
    pub name: String,
    /// Our pseudonym there, to tell our own removal from another's.
    pub our_pseudonym: String,
}

/// Minimal identity cache for dashboard display.
#[derive(Clone)]
pub(crate) struct CachedIdentity {
    pub public_key: String,
    pub display_name: String,
}

impl App {
    /// Construct a new App with all required state.
    pub fn new(
        client: Arc<DaemonClient>,
        config: &rekindle_client::config::UserConfig,
        theme: ThemeManager,
        keymap: KeymapStore,
    ) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let use_unicode = theme.use_unicode();
        let animations = config.tui.animations;

        let tabs = vec![
            Tab {
                label: "Dashboard".into(),
                id: "dashboard".into(),
                unread: 0,
            },
            Tab {
                label: "Communities".into(),
                id: "communities".into(),
                unread: 0,
            },
            Tab {
                label: "DMs".into(),
                id: "dms".into(),
                unread: 0,
            },
            Tab {
                label: "Friends".into(),
                id: "friends".into(),
                unread: 0,
            },
            Tab {
                label: "Activity".into(),
                id: "activity".into(),
                unread: 0,
            },
        ];

        Self {
            should_quit: false,
            action_tx,
            action_rx,
            client,
            theme,
            keymap,
            nav: Navigator::new(tabs, use_unicode),
            notifications: NotificationStack::new(),
            search: SearchOverlay::new(),
            intent: IntentOverlay::default(),
            loading_spinner: Spinner::new(animations, use_unicode),
            node_was_connected: false,
            network_bars: 0,
            clipboard: None,
            clipboard_clear_at: None,
            idle_frames: 0,
            cached_communities: Vec::new(),
            cached_identity: None,
            cursor: None,
            kept_cursor: None,
        }
    }

    /// Main event loop. Runs until quit.
    ///
    /// Two sources:
    /// 1. Terminal events (keys, mouse, resize, tick, render)
    /// 2. The host's stream frames (plan D3.2), subscribed from `from` by
    ///    the caller; `InvalidSession` reloads (`stream`)
    pub async fn run(
        &mut self,
        tui: &mut Tui,
        mut event_rx: Option<
            tokio::sync::mpsc::Receiver<rekindle_types::subscription_events::StreamFrame>,
        >,
        from: Option<rekindle_types::subscription_events::Cursor>,
    ) -> anyhow::Result<()> {
        self.cursor = from;
        self.kept_cursor = from;
        // Set identity from daemon if available
        if let Some(ref id) = self.cached_identity {
            self.nav
                .dashboard_mut()
                .set_identity(&id.public_key, &id.display_name);
        }
        self.loading_spinner.set_label("Loading dashboard...");
        self.loading_spinner.start();
        self.load_dashboard_data();
        self.load_skin();

        loop {
            // Biased, terminal first: a busy host stream cannot hold off a
            // key (quit among them), and the terminal cannot starve the
            // stream, because the loop drains it below on every pass.
            tokio::select! {
                biased;
                // Terminal events: keys, mouse, resize, tick, render
                event = tui.event_rx.recv() => {
                    match event {
                        Some(event) => {
                            if let Some(action) = self.event_to_action(event) {
                                let _ = self.action_tx.send(action);
                            }
                        }
                        None => break,
                    }
                }
                // The host's stream frames: real-time updates
                Some(frame) = async {
                    match event_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => self.handle_frame(frame).await,
            }
            self.keep_cursor();

            // Drain any buffered terminal events
            while let Ok(event) = tui.event_rx.try_recv() {
                if let Some(action) = self.event_to_action(event) {
                    let _ = self.action_tx.send(action);
                }
            }

            // Process all pending actions
            while let Ok(action) = self.action_rx.try_recv() {
                self.process_action(action, tui)?;
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    /// Look up a cached community name by anchor key.
    pub(crate) fn community_name<'a>(&'a self, anchor: &'a str) -> &'a str {
        self.cached_communities
            .iter()
            .find(|c| c.anchor == anchor)
            .map_or(anchor, |c| c.name.as_str())
    }

    /// Whether `pseudonym` is ours in `anchor`.
    pub(crate) fn is_own_pseudonym(&self, anchor: &str, pseudonym: &str) -> bool {
        self.cached_communities
            .iter()
            .any(|c| c.anchor == anchor && c.our_pseudonym == pseudonym)
    }

    /// The toast for a kick or ban: ours names the community and the
    /// remover (plan E3.2 S9 own removal), another's the member.
    pub(crate) fn removal_toast(
        &self,
        community: &str,
        target: &str,
        removed_by_name: Option<&str>,
        reason: Option<&str>,
        verb: &str,
    ) -> String {
        if !self.is_own_pseudonym(community, target) {
            return format!("{} was {verb}", crate::helpers::abbreviate_key(target));
        }
        let by = removed_by_name
            .map(|n| format!(" by {n}"))
            .unwrap_or_default();
        let why = reason
            .filter(|r| !r.is_empty())
            .map(|r| format!(": {r}"))
            .unwrap_or_default();
        format!(
            "You were {verb} from {}{by}{why}",
            self.community_name(community)
        )
    }
}

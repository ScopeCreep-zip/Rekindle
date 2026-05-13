//! TUI application — TEA main loop and state owner.

pub mod commands;
pub mod events;
pub mod reducer;
pub mod render;

use std::sync::Arc;
use tokio::sync::mpsc;

use super::action::{Action, ToastLevel};
use super::components::confirm_dialog::ConfirmDialogState;
use super::components::search_overlay::SearchOverlay;
use super::components::spinner::Spinner;
use super::components::tab_bar::state::Tab;
use super::components::toast::NotificationStack;
use super::keybinds::KeymapStore;
use super::navigator::Navigator;
use super::terminal::Tui;
use super::theme::ThemeManager;
use crate::v2::transport::DaemonClient;

pub struct App {
    pub(crate) should_quit: bool,
    pub(crate) action_tx: mpsc::UnboundedSender<Action>,
    pub(crate) action_rx: mpsc::UnboundedReceiver<Action>,
    pub(crate) client: Arc<DaemonClient>,
    #[allow(dead_code)]
    pub(crate) config: Arc<crate::v2::config::schema::Config>,
    pub(crate) theme: ThemeManager,
    pub(crate) keymap: KeymapStore,
    pub(crate) nav: Navigator,
    pub(crate) notifications: NotificationStack,
    pub(crate) search_overlay: SearchOverlay,
    pub(crate) confirm: ConfirmDialogState,
    pub(crate) loading_spinner: Spinner,
    pub(crate) pending_confirm_action: Option<Action>,
    pub(crate) node_was_connected: bool,
    pub(crate) clipboard: Option<arboard::Clipboard>,
    pub(crate) clipboard_clear_at: Option<std::time::Instant>,
    pub(crate) idle_frames: u32,
    pub(crate) cached_peer_count: usize,
    pub(crate) cached_communities: Vec<CachedCommunity>,
    pub(crate) cached_identity: Option<CachedIdentity>,
    /// Project-wide search engine (fff). Always initialized at TUI startup.
    pub(crate) search: Option<crate::v2::search::RekindleSearch>,
    /// Persistent notification rails — community/channel (top) and system (bottom).
    pub(crate) rails: super::components::notification_rail::NotificationRails,
    /// File content search overlay (Ctrl+G) — searches inside project files via fff.
    pub(crate) file_content_search: super::components::file_content_search::FileContentSearch,
    /// Deferred session restore — (community, Option<channel>). Validated after
    /// CommunityListLoaded arrives. None after consumed or if no saved state.
    pub(crate) pending_session_restore: Option<(String, Option<String>)>,
    /// Last time fff search was invoked from the quick switcher overlay.
    /// Used for 50ms debounce to prevent input lag on large projects.
    pub(crate) last_search_query_at: Option<std::time::Instant>,
}

#[derive(Clone)]
pub(crate) struct CachedCommunity {
    pub governance_key: String,
    pub name: String,
}

#[derive(Clone)]
pub(crate) struct CachedIdentity {
    pub public_key: String,
    pub display_name: String,
}

impl App {
    pub fn new(
        client: Arc<DaemonClient>,
        config: crate::v2::config::schema::Config,
        theme: ThemeManager,
        keymap: KeymapStore,
    ) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let use_unicode = theme.use_unicode();
        let animations = config.tui.animations;

        let tabs = vec![
            Tab { label: "Dashboard".into(), id: "dashboard".into(), unread: 0 },
            Tab { label: "Communities".into(), id: "communities".into(), unread: 0 },
            Tab { label: "DMs".into(), id: "dms".into(), unread: 0 },
            Tab { label: "Friends".into(), id: "friends".into(), unread: 0 },
        ];

        Self {
            should_quit: false, action_tx, action_rx,
            client, config: Arc::new(config), theme, keymap,
            nav: Navigator::new(tabs, use_unicode),
            notifications: NotificationStack::new(),
            search_overlay: SearchOverlay::new(),
            confirm: ConfirmDialogState::new(),
            loading_spinner: Spinner::new(animations, use_unicode),
            pending_confirm_action: None,
            node_was_connected: false,
            clipboard: None, clipboard_clear_at: None,
            idle_frames: 0, cached_peer_count: 0,
            cached_communities: Vec::new(), cached_identity: None,
            search: None,
            rails: super::components::notification_rail::NotificationRails::new(),
            file_content_search: super::components::file_content_search::FileContentSearch::new(),
            pending_session_restore: None,
            last_search_query_at: None,
        }
    }

    /// Main event loop — 3-source select: terminal + subscription + fallback refresh.
    ///
    /// Reconnection: when the daemon event channel closes, the loop enters
    /// a reconnection state. The subscription select branch becomes a
    /// reconnection timer with exponential backoff. On successful reconnect,
    /// `self.client` and `event_rx` are replaced in-place and subscriptions
    /// are re-established. The loop never breaks on daemon disconnect — only
    /// on terminal close or explicit quit.
    pub async fn run(
        &mut self,
        tui: &mut Tui,
        mut event_rx: Option<mpsc::Receiver<rekindle_types::subscription_events::SubscriptionEvent>>,
    ) -> anyhow::Result<()> {
        if let Some(ref id) = self.cached_identity {
            self.nav.dashboard_mut().set_identity(&id.public_key, &id.display_name);
        }

        // Initialize fff project-wide search from cwd
        let project_root = std::env::current_dir()
            .map_or_else(|_| ".".to_string(), |p| p.to_string_lossy().to_string());
        match crate::v2::search::RekindleSearch::init(&project_root, false) {
            Ok(s) => {
                // Wait briefly for the initial scan so the first quick switcher
                // open has file results. Non-blocking: 500ms max, then proceed
                // with partial results (scan continues in background).
                let _scan_complete = s.wait_for_scan(std::time::Duration::from_millis(500));
                tracing::info!(root = %project_root, "fff search initialized");
                self.search = Some(s);
            }
            Err(e) => {
                tracing::warn!(error = %e, "fff search init failed");
            }
        }

        // Restore saved session state (last tab, community, channel)
        let saved_session = super::session_state::load();
        if let Some(ref tab) = saved_session.active_tab {
            self.nav.tab_bar.select_by_id(tab);
        }

        self.loading_spinner.set_label("Loading dashboard...");
        self.loading_spinner.start();
        self.load_dashboard_data();

        // Deferred session navigation — stored here, validated after CommunityListLoaded
        // arrives in the reducer. If the community no longer exists (user was kicked
        // between sessions), the navigation is silently dropped rather than showing
        // a broken view. The reducer checks `cached_communities` before navigating.
        self.pending_session_restore = match (saved_session.active_community, saved_session.active_channel) {
            (Some(community), Some(channel)) => Some((community, Some(channel))),
            (Some(community), None) => Some((community, None)),
            _ => None,
        };

        if event_rx.is_some() {
            if let Err(e) = self.client.subscribe_all().await {
                tracing::warn!(error = %e, "event subscription failed — TUI will use polling only");
            }
        }

        let mut fallback_interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
        fallback_interval.tick().await;

        // Reconnection state: None = connected, Some(instant) = next reconnect attempt time
        let mut reconnect_at: Option<tokio::time::Instant> = None;
        let mut reconnect_delay = tokio::time::Duration::from_millis(500);
        let mut reconnect_attempt: u32 = 0;

        loop {
            tokio::select! {
                event = tui.event_rx.recv() => {
                    match event {
                        Some(event) => {
                            if let Some(action) = self.event_to_action(event) {
                                let _ = self.action_tx.send(action);
                            }
                        }
                        None => break, // terminal event loop ended
                    }
                }

                // When connected: receive subscription events
                // When disconnected: this branch is pending (event_rx is None)
                sub_result = async {
                    match event_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Some(sub_event) = sub_result {
                        if let Some(action) = self.subscription_event_to_action(sub_event) {
                            let _ = self.action_tx.send(action);
                        }
                    } else {
                        // Daemon connection lost — enter reconnection state
                        event_rx = None;
                        self.node_was_connected = false;
                        reconnect_at = Some(tokio::time::Instant::now() + reconnect_delay);
                        reconnect_attempt = 0;
                        self.notifications.push(
                            "Daemon disconnected — reconnecting...".into(),
                            ToastLevel::Warning,
                        );
                        self.rails.set(super::components::notification_rail::RailSignal {
                            id: "system:daemon_disconnected".into(),
                            scope: super::components::notification_rail::SignalScope::System,
                            text: "⚠ Daemon disconnected — reconnecting...".into(),
                            priority: super::components::notification_rail::SignalPriority::Critical,
                            dismissible: false,
                        });
                        tracing::warn!("daemon event channel closed");
                    }
                }

                // Reconnection timer — fires only when disconnected
                () = async {
                    match reconnect_at {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => {
                    reconnect_attempt += 1;
                    tracing::debug!(attempt = reconnect_attempt, "attempting daemon reconnect");

                    match DaemonClient::connect().await {
                        Ok(mut new_client) => {
                            let new_event_rx = new_client.take_event_receiver();

                            // Re-subscribe to all events
                            if let Err(e) = new_client.subscribe_all().await {
                                tracing::warn!(error = %e, "reconnect subscribe failed");
                            }

                            // Replace client and event channel in-place
                            self.client = Arc::new(new_client);
                            event_rx = new_event_rx;
                            reconnect_at = None;
                            reconnect_delay = tokio::time::Duration::from_millis(500);
                            self.node_was_connected = true;

                            self.notifications.push(
                                "Daemon reconnected".into(),
                                ToastLevel::Success,
                            );
                            self.rails.remove("system:daemon_disconnected");
                            tracing::info!(attempt = reconnect_attempt, "daemon reconnected");

                            // Reload all data with the new connection
                            self.load_dashboard_data();
                        }
                        Err(e) => {
                            tracing::debug!(attempt = reconnect_attempt, error = %e, "reconnect failed");
                            reconnect_delay = (reconnect_delay * 2).min(tokio::time::Duration::from_secs(15));
                            // Add jitter to prevent thundering herd when multiple clients reconnect
                            let nanos = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .subsec_nanos();
                            let jitter = tokio::time::Duration::from_millis(u64::from(nanos % 500));
                            reconnect_at = Some(tokio::time::Instant::now() + reconnect_delay + jitter);

                            if reconnect_attempt >= 20 {
                                self.notifications.push(
                                    "Daemon unreachable — restart with: rekindle node start".into(),
                                    ToastLevel::Error,
                                );
                                reconnect_at = None; // stop trying
                            }
                        }
                    }
                }

                _ = fallback_interval.tick() => {
                    self.load_dashboard_data();
                }
            }

            // Drain buffered terminal events
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
                // Save TUI session state on quit
                let session = super::session_state::TuiSessionState {
                    active_tab: self.nav.tab_bar.selected_id().map(str::to_string),
                    active_community: match self.nav.current_view_kind() {
                        crate::v2::views::ViewKind::ChannelWatch { community, .. }
                        | crate::v2::views::ViewKind::CommunityInfo { community }
                        | crate::v2::views::ViewKind::VoiceSession { community, .. } => Some(community.clone()),
                        _ => None,
                    },
                    active_channel: match self.nav.current_view_kind() {
                        crate::v2::views::ViewKind::ChannelWatch { channel, .. } => Some(channel.clone()),
                        _ => None,
                    },
                    sidebar_visible: true, // sidebar state is view-internal; save true as safe default
                };
                super::session_state::save(&session);
                break;
            }
        }
        Ok(())
    }

    pub(crate) fn community_name<'a>(&'a self, governance_key: &'a str) -> &'a str {
        self.cached_communities.iter()
            .find(|c| c.governance_key == governance_key)
            .map_or(governance_key, |c| c.name.as_str())
    }
}

//! The TUI state machine — select loop, drain, effect execution, deadlines.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time;

use rekindle_types::subscription_events::SubscriptionEvent;

use super::data_requirements::{required_data, requirements_to_effects};
use super::effects::{Effect, EffectContext};
use super::events::{DaemonEvent, TerminalEvent};
use super::idle::{self, IdleTier};
use super::process;
use super::state::navigation::ViewKind;
use super::state::render_caches::RenderCaches;
use super::state::session;
use super::state::TuiState;
use super::terminal::Tui;
use super::theme::ThemeManager;
use super::keybinds::KeymapStore;
use crate::v2::views;

pub async fn run(
    mut tui: Tui,
    client: Arc<rekindle_client::DaemonClient>,
    event_rx: mpsc::Receiver<SubscriptionEvent>,
    config: &crate::v2::config::schema::TuiConfig,
) -> anyhow::Result<()> {
    let now = Instant::now();
    let wall = wall_clock_ms();
    let mut state = TuiState::new(config.animations, true, now, wall);
    let mut caches = RenderCaches::new();

    let mut theme = ThemeManager::load(&config.theme);
    let keymap = KeymapStore::load()?;
    let mut resolved_theme: Option<ThemeManager> = None;
    let mut resolved_theme_key: Option<String> = None;

    state.search_engine = crate::v2::search::RekindleSearch::init(".", false).ok();
    if let Some(ref engine) = state.search_engine {
        let _ = engine.wait_for_scan(std::time::Duration::from_millis(500));
        tracing::info!(scanning = engine.is_scanning(), "search engine initialized");
    }

    let persisted = session::load();
    state.session = super::state::session::SessionState::restore(&persisted);
    if let Some(ref tab) = persisted.active_tab {
        state.nav.tab_bar.select_by_id(tab);
    }
    state.nav.sidebar_visible = persisted.sidebar_visible;
    state.timezone = persisted.timezone;

    let (daemon_tx, mut daemon_rx) = mpsc::channel::<DaemonEvent>(4096);
    let (internal_tx, mut internal_rx) = mpsc::channel::<super::events::InternalEvent>(256);

    let sub_daemon_tx = daemon_tx.clone();
    let mut bridge_handle: Option<JoinHandle<()>> = Some(tokio::spawn(async move {
        tracing::info!("tui: bridge task started — forwarding subscription events");
        let mut event_rx = event_rx;
        let mut count: u64 = 0;
        while let Some(event) = event_rx.recv().await {
            count += 1;
            tracing::info!(count, event_type = %event_type_label(&event), "tui: bridge received subscription event");
            if sub_daemon_tx.send(DaemonEvent::Subscription(event)).await.is_err() {
                tracing::error!("tui: bridge daemon_tx send FAILED — channel closed");
                break;
            }
        }
        tracing::warn!(total_events = count, "tui: bridge task ending — event_rx closed, sending ConnectionLost");
        let _ = sub_daemon_tx.send(DaemonEvent::ConnectionLost).await;
    }));

    let mut effect_ctx = EffectContext {
        client: Arc::clone(&client),
        daemon_tx: daemon_tx.clone(),
        internal_tx: internal_tx.clone(),
        clipboard: arboard::Clipboard::new().ok(),
    };

    // Subscribe to ALL daemon events before loading any data.
    // Without this, no subscription events (DMs, channel messages, typing,
    // friend requests, presence) are delivered to the TUI.
    if let Err(e) = client.subscribe_all().await {
        tracing::error!(error = %e, "tui: subscribe_all FAILED at startup");
    } else {
        tracing::info!("tui: subscribed to all daemon events");
    }

    state.ephemeral.spinner.set_label("Loading dashboard...");
    state.ephemeral.spinner.start();

    tracing::info!(
        restored_tab = ?persisted.active_tab,
        restored_peer = ?persisted.dm_selected_peer,
        sidebar = persisted.sidebar_visible,
        "tui: startup — session restored, loading dashboard data"
    );
    let initial_reqs = required_data(&ViewKind::Dashboard, &state);
    tracing::info!(req_count = initial_reqs.len(), "tui: startup — initial data requirements");
    let initial_effects = requirements_to_effects(initial_reqs, &mut state.in_flight, state.now);
    let deferred = execute_effects(initial_effects, &mut state, &mut caches, &mut effect_ctx, &mut theme, false);
    if !deferred.is_empty() {
        let _ = execute_effects(deferred, &mut state, &mut caches, &mut effect_ctx, &mut theme, false);
    }

    let mut render_interval = time::interval(Duration::from_millis(state.idle.render_interval().as_millis() as u64));
    let mut tick_interval = time::interval(Duration::from_millis(state.idle.tick_interval().as_millis() as u64));
    let mut status_interval = time::interval(state.idle.status_poll_interval());
    let mut full_refresh_interval = time::interval(Duration::from_secs(30));

    render_interval.tick().await;
    tick_interval.tick().await;
    status_interval.tick().await;
    full_refresh_interval.tick().await;

    let mut reconnect_at: Option<Instant> = None;
    let mut reconnect_attempt: u32 = 0;
    const MAX_RECONNECT_ATTEMPTS: u32 = 20;
    const RECONNECT_BASE_MS: u64 = 500;
    const RECONNECT_MAX_MS: u64 = 15_000;

    loop {
        state.now = Instant::now();
        state.wall_clock_ms = wall_clock_ms();

        let reconnecting = reconnect_at.is_some();
        let mut effects: Vec<Effect> = Vec::new();

        tokio::select! {
            biased;

            Some(event) = tui.event_rx.recv() => {
                let te = convert_terminal_event(event);
                effects.extend(process::input::process_input(&te, &mut state, &keymap, &mut caches));
                while let Ok(more) = tui.event_rx.try_recv() {
                    let te = convert_terminal_event(more);
                    effects.extend(process::input::process_input(&te, &mut state, &keymap, &mut caches));
                }
                state.idle = IdleTier::Active;
                state.ephemeral.idle_frames = 0;
                state.ephemeral.last_input_at = state.now;
                state.render_needed = true;
            }

            Some(event) = daemon_rx.recv() => {
                tracing::info!("tui: daemon_rx received event");
                if matches!(&event, DaemonEvent::ConnectionLost) {
                    reconnect_at = Some(state.now + Duration::from_millis(RECONNECT_BASE_MS));
                    reconnect_attempt = 0;
                }
                effects.extend(process::daemon::process_daemon(&event, &mut state));
                while let Ok(more) = daemon_rx.try_recv() {
                    if matches!(&more, DaemonEvent::ConnectionLost) {
                        reconnect_at = Some(state.now + Duration::from_millis(RECONNECT_BASE_MS));
                        reconnect_attempt = 0;
                    }
                    effects.extend(process::daemon::process_daemon(&more, &mut state));
                }
                state.render_needed = true;
            }

            Some(event) = internal_rx.recv() => {
                effects.extend(process::internal::process_internal(&event, &mut state));
                state.render_needed = true;
            }

            _ = render_interval.tick() => {
                if state.render_needed {
                    tracing::trace!(view = ?state.nav.current_view(), "tui: render tick — rendering");
                    let active_theme = resolve_theme(
                        &state, &theme,
                        &mut resolved_theme, &mut resolved_theme_key,
                    );
                    render(&state, &mut tui, active_theme, &keymap, &mut caches);
                    state.render_needed = false;
                    state.ephemeral.idle_frames = state.ephemeral.idle_frames.saturating_add(1);
                }
            }

            _ = tick_interval.tick() => {
                effects.extend(process::timer::process_tick(&mut state));
            }

            _ = status_interval.tick() => {
                if state.idle.should_poll_status() {
                    let reqs = required_data(&ViewKind::Dashboard, &state);
                    let status_only: Vec<_> = reqs.into_iter()
                        .filter(|r| matches!(r, super::data_requirements::DataRequirement::Status))
                        .collect();
                    effects.extend(requirements_to_effects(status_only, &mut state.in_flight, state.now));
                }
            }

            _ = full_refresh_interval.tick() => {
                if state.idle.should_full_refresh() {
                    let reqs = required_data(&ViewKind::Dashboard, &state);
                    effects.extend(requirements_to_effects(reqs, &mut state.in_flight, state.now));
                }
            }

            _ = async {
                match reconnect_at {
                    Some(at) => tokio::time::sleep_until(tokio::time::Instant::from_std(at)).await,
                    None => std::future::pending::<()>().await,
                }
            }, if reconnect_at.is_some() => {
                reconnect_at = None;
                reconnect_attempt += 1;

                if reconnect_attempt > MAX_RECONNECT_ATTEMPTS {
                    state.ephemeral.toasts.push(
                        "Reconnection failed \u{2014} restart daemon with: rekindle node start".into(),
                        super::state::ephemeral::ToastLevel::Error,
                        state.now,
                    );
                } else {
                    match rekindle_client::DaemonClient::connect().await {
                        Ok(mut new_client) => {
                            if let Some(handle) = bridge_handle.take() {
                                handle.abort();
                            }

                            let new_event_rx = new_client.take_event_receiver();
                            let new_client = Arc::new(new_client);

                            effect_ctx.client = Arc::clone(&new_client);

                            let (new_daemon_tx, new_daemon_rx) = mpsc::channel::<DaemonEvent>(4096);
                            effect_ctx.daemon_tx = new_daemon_tx.clone();
                            daemon_rx = new_daemon_rx;

                            if let Some(rx) = new_event_rx {
                                let btx = new_daemon_tx;
                                bridge_handle = Some(tokio::spawn(async move {
                                    tracing::info!("tui: reconnect bridge task started");
                                    let mut rx = rx;
                                    let mut count: u64 = 0;
                                    while let Some(event) = rx.recv().await {
                                        count += 1;
                                        if btx.send(DaemonEvent::Subscription(event)).await.is_err() {
                                            tracing::error!("tui: reconnect bridge daemon_tx send FAILED");
                                            break;
                                        }
                                    }
                                    tracing::warn!(total_events = count, "tui: reconnect bridge ending — sending ConnectionLost");
                                    let _ = btx.send(DaemonEvent::ConnectionLost).await;
                                }));
                            } else {
                                bridge_handle = None;
                            }

                            let _ = new_client.subscribe_all().await;

                            state.ephemeral.rails.remove("system:daemon_disconnected");
                            state.ephemeral.rails.clear_scope(super::state::ephemeral::SignalScope::System);
                            state.node_connected = true;
                            reconnect_attempt = 0;

                            let view = state.nav.current_view().clone();
                            let reqs = required_data(&view, &state);
                            effects.extend(requirements_to_effects(reqs, &mut state.in_flight, state.now));

                            let dash_reqs = required_data(&ViewKind::Dashboard, &state);
                            effects.extend(requirements_to_effects(dash_reqs, &mut state.in_flight, state.now));

                            state.ephemeral.toasts.push(
                                "Reconnected to daemon".into(),
                                super::state::ephemeral::ToastLevel::Success,
                                state.now,
                            );
                            state.render_needed = true;
                        }
                        Err(e) => {
                            let backoff_ms = (RECONNECT_BASE_MS * 2u64.pow(reconnect_attempt.min(5)))
                                .min(RECONNECT_MAX_MS);
                            let jitter = state.wall_clock_ms % 500;
                            reconnect_at = Some(state.now + Duration::from_millis(backoff_ms + jitter));

                            tracing::debug!(
                                attempt = reconnect_attempt,
                                next_ms = backoff_ms + jitter,
                                error = %e,
                                "reconnection failed, retrying"
                            );
                        }
                    }
                }
            }
        }

        effects.extend(check_deadlines(&mut state));

        if !effects.is_empty() {
            tracing::debug!(effect_count = effects.len(), "tui: executing loop effects");
        }
        let deferred = execute_effects(effects, &mut state, &mut caches, &mut effect_ctx, &mut theme, reconnecting);
        if !deferred.is_empty() {
            tracing::debug!(deferred_count = deferred.len(), "tui: executing deferred effects");
            let _ = execute_effects(deferred, &mut state, &mut caches, &mut effect_ctx, &mut theme, reconnecting);
        }

        let new_tier = IdleTier::from_elapsed(state.ephemeral.last_input_at, state.now);
        if new_tier != state.idle {
            state.idle = new_tier;
            idle::adjust_interval(&mut render_interval, new_tier.render_interval());
            idle::adjust_interval(&mut tick_interval, new_tier.tick_interval());
            idle::adjust_interval(&mut status_interval, new_tier.status_poll_interval());
        }

        if state.should_quit {
            let persisted = state.session.to_persisted(
                state.nav.tab_bar.selected_id(),
                match state.nav.current_view() {
                    ViewKind::ChannelWatch { community, .. }
                    | ViewKind::CommunityInfo { community }
                    | ViewKind::VoiceSession { community, .. } => Some(community.as_str()),
                    _ => None,
                },
                match state.nav.current_view() {
                    ViewKind::ChannelWatch { channel, .. } => Some(channel.as_str()),
                    _ => None,
                },
                state.nav.sidebar_visible,
                state.timezone,
            );
            session::save(&persisted);
            break;
        }
    }

    if let Some(handle) = bridge_handle.take() {
        handle.abort();
    }
    drop(tui);
    Ok(())
}

fn execute_effects(
    effects: Vec<Effect>,
    state: &mut TuiState,
    caches: &mut RenderCaches,
    ctx: &mut EffectContext,
    theme: &mut ThemeManager,
    reconnecting: bool,
) -> Vec<Effect> {
    let mut deferred = Vec::new();

    for effect in effects {
        match effect {
            Effect::IpcRequest { request_id, request } => {
                let kind = super::state::in_flight::request_kind_from(&request);
                if reconnecting {
                    tracing::debug!(request_id, kind = ?kind, "tui: effect deferred (reconnecting)");
                    state.in_flight.push_deferred(request_id, request);
                    continue;
                }
                if !state.in_flight.try_reserve(&kind) {
                    tracing::debug!(request_id, kind = ?kind, "tui: effect deferred (credit ceiling)");
                    state.in_flight.push_deferred(request_id, request);
                    continue;
                }
                tracing::info!(request_id, kind = ?kind, "tui: dispatching IPC request");
                spawn_ipc_request(request_id, request, ctx);
            }

            Effect::IpcSubscribe { filters } => {
                if reconnecting {
                    continue;
                }
                let client = Arc::clone(&ctx.client);
                tokio::spawn(async move {
                    let _ = client.request_ok(
                        rekindle_types::daemon::DaemonRequest::Lifecycle(
                            rekindle_types::daemon::LifecycleRequest::Subscribe { filters },
                        ),
                    ).await;
                });
            }

            Effect::Navigate(view_kind) => {
                tracing::info!(view = ?view_kind, "tui: Navigate effect");
                let old_tag = state.nav.current_view().tag();
                caches.click_targets.remove(&old_tag);
                state.nav.push_view(view_kind.clone());
                state.nav.sync_tab_to_view(&view_kind);
                state.nav.input_mode = false;
                {
                    use super::focus::FocusId;
                    let slots = match &view_kind {
                        ViewKind::ChannelWatch { .. } => vec![
                            FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox, FocusId::PeerList, FocusId::ThreadPanel,
                        ],
                        ViewKind::DmInbox => vec![FocusId::DmList, FocusId::MessageList, FocusId::InputBox],
                        ViewKind::DmThread { .. } => vec![FocusId::MessageList, FocusId::InputBox],
                        ViewKind::Dashboard => vec![
                            FocusId::DashIdentity, FocusId::DashNode, FocusId::ChannelTree, FocusId::FriendList,
                        ],
                        ViewKind::Doctor => vec![FocusId::DoctorList],
                        ViewKind::FriendList => vec![FocusId::FriendList],
                        ViewKind::CommunityInfo { .. } => vec![FocusId::CommunityInfoPanel],
                        ViewKind::IdentitySettings => vec![FocusId::IdentitySettings],
                        ViewKind::VoiceSession { .. } => vec![FocusId::VoiceParticipants],
                        ViewKind::Moderation { .. } | ViewKind::Invite { .. }
                        | ViewKind::Events { .. } | ViewKind::Onboarding { .. }
                        | ViewKind::FilePreview { .. } => vec![FocusId::MessageList],
                    };
                    state.nav.focus_ring.set_slots(slots);
                }
                if matches!(&view_kind, ViewKind::FilePreview { .. }) {
                    state.ephemeral.file_preview_scroll_offset = 0;
                }
                let reqs = required_data(&view_kind, state);
                deferred.extend(requirements_to_effects(reqs, &mut state.in_flight, state.now));
            }

            Effect::SetClipboard(text) => {
                if let Some(ref mut cb) = ctx.clipboard {
                    let _ = cb.set_text(&text);
                    state.ephemeral.clipboard_clear_at = Some(state.now + Duration::from_secs(30));
                }
            }

            Effect::ClearClipboard => {
                if let Some(ref mut cb) = ctx.clipboard {
                    let _ = cb.set_text("");
                }
                state.ephemeral.clipboard_clear_at = None;
            }

            Effect::SetTheme { name } => {
                *theme = ThemeManager::load(&name);
                state.render_needed = true;
                state.ephemeral.toasts.push(
                    format!("Theme: {}", theme.name()),
                    super::state::ephemeral::ToastLevel::Success,
                    state.now,
                );
            }

            Effect::OsNotify { title, body } => {
                let tx = ctx.internal_tx.clone();
                let _ = tx.try_send(super::events::InternalEvent::Toast {
                    message: if title.is_empty() { body } else { format!("{title}: {body}") },
                    level: super::state::ephemeral::ToastLevel::Info,
                });
            }

            Effect::SaveSession => {
                let persisted = state.session.to_persisted(
                    state.nav.tab_bar.selected_id(),
                    None,
                    None,
                    state.nav.sidebar_visible,
                    state.timezone,
                );
                session::save(&persisted);
            }

            Effect::Quit => {
                state.should_quit = true;
            }
        }
    }

    deferred
}

fn spawn_ipc_request(request_id: u64, request: rekindle_types::daemon::DaemonRequest, ctx: &EffectContext) {
    let client = Arc::clone(&ctx.client);
    let tx = ctx.daemon_tx.clone();
    tokio::spawn(async move {
        match client.request(request).await {
            Ok(response) => {
                tracing::info!(request_id, "tui: IPC response ok, forwarding to daemon_tx");
                if tx.send(DaemonEvent::CommandResult {
                    request_id,
                    response,
                }).await.is_err() {
                    tracing::error!(request_id, "tui: daemon_tx send FAILED — receiver dropped");
                }
            }
            Err(e) => {
                tracing::warn!(request_id, error = %e, "tui: IPC request error");
                let _ = tx.send(DaemonEvent::CommandFailed {
                    request_id,
                    error: e.to_string(),
                }).await;
            }
        }
    });
}

fn check_deadlines(state: &mut TuiState) -> Vec<Effect> {
    let mut effects = vec![];

    state.ephemeral.typing_indicators.retain(|_, since| {
        state.now.duration_since(*since) < Duration::from_secs(5)
    });

    for (_, thread) in state.dm.threads.iter_mut() {
        if let Some(since) = thread.typing_since {
            if state.now.duration_since(since) > Duration::from_secs(5) {
                thread.is_typing = false;
                thread.typing_since = None;
                state.render_needed = true;
            }
        }
    }

    if let Some(deadline) = state.ephemeral.clipboard_clear_at {
        if state.now >= deadline {
            effects.push(Effect::ClearClipboard);
        }
    }

    state.ephemeral.toasts.tick(state.now);

    let timed_out: Vec<u64> = state.in_flight.requests_iter()
        .filter(|(_, meta)| {
            let timeout = match &meta.kind {
                super::state::in_flight::RequestKind::Send => Duration::from_secs(180),
                _ => Duration::from_secs(30),
            };
            state.now.duration_since(meta.sent_at) > timeout
        })
        .map(|(id, _)| id)
        .collect();

    for id in timed_out {
        if let Some(meta) = state.in_flight.remove_request(id) {
            state.in_flight.release_credit(&meta.kind);
            state.ephemeral.toasts.push(
                format!("Request timed out: {:?}", meta.kind),
                super::state::ephemeral::ToastLevel::Warning,
                state.now,
            );
        }
    }

    while state.in_flight.has_deferred() {
        if let Some(&(ref _req_id, ref request)) = state.in_flight.peek_deferred() {
            let kind = super::state::in_flight::request_kind_from(request);
            if state.in_flight.try_reserve(&kind) {
                if let Some((req_id, request)) = state.in_flight.pop_deferred() {
                    effects.push(Effect::IpcRequest { request_id: req_id, request });
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }

    effects
}

fn render(state: &TuiState, tui: &mut Tui, theme: &ThemeManager, keymap: &KeymapStore, caches: &mut RenderCaches) {
    let _ = tui.draw(|frame| {
        use ratatui::layout::{Constraint, Layout};
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};
        use super::components::{confirm_dialog, help_bar, status_bar, tab_bar};
        use super::keybinds::KeymapContext;
        use super::state::navigation::{OverlayState, ViewKind};

        let full = frame.area();

        let community_rail_h = state.ephemeral.rails.community_rail_height();
        let system_rail_h = state.ephemeral.rails.system_rail_height();
        let spinner_h = if state.ephemeral.spinner.is_active() { 1u16 } else { 0 };

        let [tab_area, community_rail_area, content_area, system_rail_area, spinner_area, status_area, help_area] =
            Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(community_rail_h),
                Constraint::Fill(1),
                Constraint::Length(system_rail_h),
                Constraint::Length(spinner_h),
                Constraint::Length(1),
                Constraint::Length(1),
            ]).areas(full);

        // Tab bar
        tab_bar::render::render(frame, tab_area, &state.nav.tab_bar, &mut caches.tab_bar_click_regions, theme);

        // Notification rails
        state.ephemeral.rails.render_community_rail(frame, community_rail_area, theme);
        state.ephemeral.rails.render_system_rail(frame, system_rail_area, theme);

        // Spinner
        state.ephemeral.spinner.render_line(frame, spinner_area, theme);

        // View content
        views::draw(state, frame, content_area, theme, caches);

        // Status bar
        let mode = if state.nav.overlay.is_some() {
            status_bar::Mode::Normal
        } else if state.nav.input_mode {
            status_bar::Mode::Insert
        } else if state.nav.search.is_some() {
            status_bar::Mode::Search
        } else {
            status_bar::Mode::Normal
        };
        let breadcrumb = match state.nav.current_view() {
            ViewKind::ChannelWatch { community, channel } => {
                format!("{} > #{channel}", state.communities.name_for(community))
            }
            ViewKind::CommunityInfo { community } => {
                state.communities.name_for(community).to_string()
            }
            ViewKind::Moderation { community } => {
                format!("Moderation: {}", state.communities.name_for(community))
            }
            ViewKind::DmThread { peer_key } => {
                state.dm.threads.get(peer_key)
                    .map(|t| t.peer_name.clone())
                    .unwrap_or_else(|| "DM".into())
            }
            _ => String::new(),
        };
        let context = if state.nav.input_mode { KeymapContext::Input } else { KeymapContext::Default };
        let keymap_hints = keymap.hint_line(context);
        let status = status_bar::StatusBarState {
            mode,
            breadcrumb,
            typing_context: String::new(),
            node_attached: state.node_connected,
            peer_count: state.cached_peer_count,
            hints: keymap_hints,
            timezone_label: state.timezone.abbreviation(),
        };
        status_bar::render(frame, status_area, &status, theme);

        // Help bar
        let help_hints = keymap.help_text(context);
        help_bar::render(frame, help_area, &help_hints, theme);

        // Overlays (on top of content)
        if let Some(ref overlay) = state.nav.overlay {
            match overlay {
                OverlayState::Help => {
                    let all_hints = keymap.help_text(KeymapContext::Default);
                    let input_hints = keymap.help_text(KeymapContext::Input);

                    let popup_width = 60u16.min(full.width.saturating_sub(4));
                    let popup_height = 20u16.min(full.height.saturating_sub(4));
                    let popup_x = full.x + (full.width.saturating_sub(popup_width)) / 2;
                    let popup_y = full.y + (full.height.saturating_sub(popup_height)) / 2;
                    let popup = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

                    frame.render_widget(Clear, popup);
                    let block = Block::bordered()
                        .title(format!(" Help \u{2014} {} ", theme.name()))
                        .border_style(Style::default().fg(theme.color("accent.primary")));
                    let inner = block.inner(popup);
                    frame.render_widget(block, popup);

                    let mut items: Vec<ListItem<'_>> = Vec::new();
                    items.push(ListItem::new(Line::from(
                        Span::styled(" NORMAL MODE", theme.style("accent")),
                    )));
                    for (combo, desc) in &all_hints {
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled(format!("  {combo:<16}"), Style::new().bold()),
                            Span::raw(*desc),
                        ])));
                    }
                    items.push(ListItem::new(Line::from("")));
                    items.push(ListItem::new(Line::from(
                        Span::styled(" INPUT MODE", theme.style("accent")),
                    )));
                    for (combo, desc) in &input_hints {
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled(format!("  {combo:<16}"), Style::new().bold()),
                            Span::raw(*desc),
                        ])));
                    }

                    items.push(ListItem::new(Line::from("")));
                    items.push(ListItem::new(Line::from(
                        Span::styled(
                            format!(" Themes: {}", super::theme::ThemeManager::available_themes().join(", ")),
                            theme.style("dim"),
                        ),
                    )));

                    frame.render_widget(List::new(items), inner);
                }
                OverlayState::Confirm => {
                    confirm_dialog::render::render(frame, content_area, &state.nav.confirm, theme);
                }
                OverlayState::EmojiPicker => {
                    state.nav.emoji_picker.draw(frame, content_area);
                }
            }
        }

        // Search overlay
        if let Some(ref search) = state.nav.search {
            search.render(frame, content_area, theme, &mut caches.search_list);
        }

        // File search overlay
        if let Some(ref fs) = state.nav.file_search {
            if fs.visible {
                let popup_width = 70u16.min(full.width.saturating_sub(4));
                let popup_height = 15u16.min(full.height.saturating_sub(4));
                let popup_x = full.x + (full.width.saturating_sub(popup_width)) / 2;
                let popup_y = full.y + (full.height.saturating_sub(popup_height)) / 2;
                let popup = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

                frame.render_widget(Clear, popup);
                let block = Block::bordered()
                    .title(format!(" File Search ({} matches) ", fs.total_matches))
                    .border_style(Style::default().fg(theme.color("accent.primary")));
                let inner = block.inner(popup);
                frame.render_widget(block, popup);

                let [fs_input_area, fs_results_area] = Layout::vertical([
                    Constraint::Length(1), Constraint::Fill(1),
                ]).areas(inner);

                let fs_input_line = Line::from(vec![
                    Span::raw(format!(" > {}", fs.query)),
                    Span::styled("\u{2588}", Style::new().dim()),
                ]);
                frame.render_widget(Paragraph::new(fs_input_line), fs_input_area);

                if fs.results.is_empty() {
                    let msg = if fs.query.is_empty() { "  Type to search files..." } else { "  No matches." };
                    frame.render_widget(Paragraph::new(msg).style(Style::new().dim()), fs_results_area);
                } else {
                    let fs_items: Vec<ListItem<'_>> = fs.results.iter().enumerate().map(|(i, r)| {
                        let selected = fs.selected_index == Some(i);
                        let style = if selected { Style::new().reversed() } else { Style::new() };
                        ListItem::new(vec![
                            Line::from(Span::styled(format!("  {}:{}", r.file_path, r.line_number), style)),
                            Line::from(Span::styled(format!("    {}", r.line_content), Style::new().dim())),
                        ])
                    }).collect();
                    frame.render_widget(List::new(fs_items), fs_results_area);
                }
            }
        }

        // Toasts (always on top of everything)
        if !state.ephemeral.toasts.is_empty() {
            state.ephemeral.toasts.render(frame, full, theme);
        }
    });
}


fn convert_terminal_event(event: super::event::Event) -> TerminalEvent {
    match event {
        super::event::Event::Key(k) => TerminalEvent::Key(k),
        super::event::Event::Mouse(m) => TerminalEvent::Mouse(m),
        super::event::Event::Resize(w, h) => TerminalEvent::Resize(w, h),
        super::event::Event::Paste(s) => TerminalEvent::Paste(s),
        super::event::Event::FocusGained => TerminalEvent::FocusGained,
        super::event::Event::FocusLost => TerminalEvent::FocusLost,
    }
}

fn resolve_theme<'a>(
    state: &TuiState,
    default: &'a ThemeManager,
    cached: &'a mut Option<ThemeManager>,
    cached_key: &mut Option<String>,
) -> &'a ThemeManager {
    // Priority 1: preview override (search hover)
    if let Some(ref preview_name) = state.theme_preview {
        let key = format!("preview:{preview_name}");
        if cached_key.as_deref() != Some(&key) {
            *cached = Some(ThemeManager::load(preview_name));
            *cached_key = Some(key);
        }
        return cached.as_ref().unwrap();
    }

    // Priority 2: per-conversation pin
    let pin_name = match state.nav.current_view() {
        ViewKind::ChannelWatch { community, .. }
        | ViewKind::CommunityInfo { community }
        | ViewKind::Moderation { community }
        | ViewKind::Invite { community }
        | ViewKind::Events { community }
        | ViewKind::Onboarding { community }
        | ViewKind::VoiceSession { community, .. } => {
            state.session.community_themes.get(community)
        }
        ViewKind::DmThread { peer_key } => {
            state.session.dm_themes.get(peer_key)
        }
        ViewKind::DmInbox => {
            state.session.dm_selected_peer.as_ref()
                .and_then(|pk| state.session.dm_themes.get(pk))
        }
        _ => None,
    };

    if let Some(name) = pin_name {
        let key = format!("pin:{name}");
        if cached_key.as_deref() != Some(&key) {
            *cached = Some(ThemeManager::load(name));
            *cached_key = Some(key);
        }
        return cached.as_ref().unwrap();
    }

    // Priority 3: user default
    if cached_key.is_some() {
        *cached = None;
        *cached_key = None;
    }
    default
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn event_type_label(event: &SubscriptionEvent) -> &'static str {
    match event {
        SubscriptionEvent::ChannelMessage(_) => "ChannelMessage",
        SubscriptionEvent::Typing(_) => "Typing",
        SubscriptionEvent::Presence(_) => "Presence",
        SubscriptionEvent::Friend(_) => "Friend",
        SubscriptionEvent::Dm(_) => "Dm",
        SubscriptionEvent::Membership(_) => "Membership",
        SubscriptionEvent::Governance(_) => "Governance",
        SubscriptionEvent::Social(_) => "Social",
        SubscriptionEvent::Voice(_) => "Voice",
        SubscriptionEvent::Crypto(_) => "Crypto",
        SubscriptionEvent::Network(_) => "Network",
        SubscriptionEvent::System(_) => "System",
        SubscriptionEvent::UnreadChanged { .. } => "UnreadChanged",
        SubscriptionEvent::BulkTransferProgress { .. } => "BulkTransferProgress",
    }
}

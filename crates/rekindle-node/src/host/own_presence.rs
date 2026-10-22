//! Our own presence, one owner on both hosts (plan E1.1): the status, the
//! status message, the game and the idle state, and the unlock's one STATUS
//! publisher (plan C7.8c). Every change is published as
//! `PresenceEvent::SelfChanged` from here, and written to the profile.
//!
//! The publisher reads the profile key at write time from the session
//! projection: a created identity's profile appears only after the
//! own-records stage, which wakes the publisher then. The teardown writes
//! Offline once the unlock's tasks have stopped; nothing else writes it.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::RwLock;
use rekindle_presence::{PresenceError, StatusPublisherDeps};
use rekindle_transport::{Session, TransportNode};
use rekindle_types::dht_layout::profile as subkeys;
use rekindle_types::dto::presence::{OwnGame, OwnPresenceView};
use rekindle_types::game_server::ServerAddress;
use rekindle_types::presence::{IdleState, SessionStatus};
use rekindle_types::subscription_events::{
    GameActivity, PresenceEvent, PresenceSnapshot, SubscriptionEvent,
};

/// How long the teardown waits for the Offline write; the write itself runs
/// on (Signal Desktop bounds its shutdown queue drain the same way).
const OFFLINE_WRITE_WAIT: Duration = Duration::from_secs(10);

/// The game we are playing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Game {
    /// Its id in the game database (0 when set by hand without one).
    pub id: u32,
    pub name: String,
    /// The game's own detail line ("map @ server"), when it reports one.
    pub server_info: Option<String>,
    pub server_address: Option<ServerAddress>,
    /// When it started, ms since the epoch.
    pub started_at_ms: u64,
}

/// What we show.
#[derive(Debug, Clone)]
struct State {
    status: SessionStatus,
    /// The status auto-away replaced, while it holds.
    pre_away: Option<SessionStatus>,
    message: String,
    game: Option<Game>,
    idle: IdleState,
}

impl State {
    fn unlocked() -> Self {
        Self {
            status: SessionStatus::Online,
            pre_away: None,
            message: String::new(),
            game: None,
            idle: IdleState::Unavailable,
        }
    }
}

/// Our presence and what it needs to publish itself. Cloned into the
/// unlock's tasks (the publisher, the idle and game stages).
#[derive(Clone)]
pub(crate) struct OwnPresence {
    state: Arc<RwLock<State>>,
    wake: Arc<tokio::sync::Notify>,
    session: Arc<RwLock<Option<Session>>>,
    transport: Option<Arc<TransportNode>>,
    journal: Arc<rekindle_events::SessionJournal>,
    clock: Arc<dyn rekindle_utils::Clock>,
}

impl OwnPresence {
    pub(crate) fn new(
        session: Arc<RwLock<Option<Session>>>,
        transport: Option<Arc<TransportNode>>,
        journal: Arc<rekindle_events::SessionJournal>,
        clock: Arc<dyn rekindle_utils::Clock>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(State::unlocked())),
            wake: Arc::new(tokio::sync::Notify::new()),
            session,
            transport,
            journal,
            clock,
        }
    }

    /// The Host's clock port (plan C9.2), for the records published
    /// beside our presence.
    pub(crate) fn clock(&self) -> &dyn rekindle_utils::Clock {
        &*self.clock
    }

    /// Start the unlock Online, with no message or game, and run the STATUS
    /// publisher in `scope`.
    pub(crate) fn start(&self, scope: &Arc<rekindle_lifecycle::SessionScope>) {
        *self.state.write() = State::unlocked();
        if self.transport.is_some() {
            let deps = Arc::new(self.clone());
            let wake = Arc::clone(&self.wake);
            scope.spawn_with_token_or_drop("status publisher", move |stop| {
                rekindle_presence::run_status_publisher(deps, wake, stop)
            });
        }
        self.changed();
    }

    /// Our status now.
    pub(crate) fn status(&self) -> SessionStatus {
        self.state.read().status
    }

    /// A status the user picked: auto-away no longer holds it.
    pub(crate) fn set_status(&self, status: SessionStatus) {
        {
            let mut state = self.state.write();
            state.status = status;
            state.pre_away = None;
            if state.idle == IdleState::AutoAway {
                state.idle = IdleState::Active;
            }
        }
        self.changed();
    }

    /// Our status message, written to profile subkey 1.
    ///
    /// # Errors
    /// The profile write failed (the message is kept and shown).
    pub(crate) async fn set_message(&self, message: String) -> Result<(), String> {
        self.state.write().message.clone_from(&message);
        self.changed();
        self.write_profile(subkeys::STATUS_MESSAGE, message.into_bytes())
            .await
    }

    /// The game we play, or none: written to profile subkey 4 as the
    /// schema's packed `GameStatus`, empty for none (plan E1.1), sealed
    /// under our friend-presence key (plan E2.5).
    ///
    /// # Errors
    /// The profile write failed (the game is kept and shown).
    pub(crate) async fn set_game(&self, game: Option<Game>) -> Result<(), String> {
        self.state.write().game.clone_from(&game);
        self.changed();
        self.write_game(game).await
    }

    /// Our presence key moved to a new epoch: STATUS and GAME_INFO are
    /// written again under it, so the friends who hold only the new key
    /// read them.
    pub(crate) async fn reseal(&self) {
        self.republish();
        let game = self.state.read().game.clone();
        if let Err(e) = self.write_game(game).await {
            tracing::warn!(error = %e, "game info not resealed");
        }
    }

    async fn write_game(&self, game: Option<Game>) -> Result<(), String> {
        let bytes = game.map_or_else(Vec::new, |game| {
            rekindle_codec::capnp_codec::presence::encode_game_status(
                &rekindle_codec::capnp_codec::presence::GameInfo {
                    game_id: game.id,
                    game_name: game.name.clone(),
                    server_info: game.server_info.clone(),
                    elapsed_seconds: self.elapsed_seconds(&game),
                    server_address: game.server_address.map(String::from),
                },
            )
        });
        let (owner, key) = self
            .presence_key()
            .ok_or("no friend-presence key yet: the game is not written in the clear")?;
        let sealed =
            rekindle_secrets::friend_presence::seal(&key, &owner, subkeys::GAME_INFO, &bytes)
                .map_err(|e| format!("seal game info: {e}"))?;
        self.write_profile(subkeys::GAME_INFO, sealed).await
    }

    /// The idle stage's reading: auto-away sets Away over Online, keeping
    /// the status it replaced.
    pub(crate) fn auto_away(&self) {
        {
            let mut state = self.state.write();
            if state.status != SessionStatus::Online || state.idle == IdleState::AutoAway {
                return;
            }
            state.pre_away = Some(state.status);
            state.status = SessionStatus::Away;
            state.idle = IdleState::AutoAway;
        }
        self.changed();
    }

    /// Activity is back: restore what auto-away replaced, or note that
    /// idle time is readable.
    pub(crate) fn active(&self) {
        {
            let mut state = self.state.write();
            match state.idle {
                IdleState::AutoAway => {
                    state.status = state.pre_away.take().unwrap_or(SessionStatus::Online);
                }
                IdleState::Unavailable => {}
                IdleState::Active => return,
            }
            state.idle = IdleState::Active;
        }
        self.changed();
    }

    /// No idle time can be read here: auto-away cannot arm.
    pub(crate) fn idle_unavailable(&self) {
        if self.state.read().idle == IdleState::Unavailable {
            return;
        }
        self.state.write().idle = IdleState::Unavailable;
        self.changed();
    }

    /// Publish the current status now (the own-records stage once our
    /// profile exists; a profile moved to a new record).
    pub(crate) fn republish(&self) {
        self.wake.notify_one();
    }

    /// What `PresenceGet` answers.
    pub(crate) fn view(&self) -> OwnPresenceView {
        let state = self.state.read().clone();
        OwnPresenceView {
            status: state.status,
            status_message: state.message,
            game: state.game.as_ref().map(|game| OwnGame {
                game_id: game.id,
                game_name: game.name.clone(),
                elapsed_seconds: self.elapsed_seconds(game),
                server_address: game.server_address.clone().map(String::from),
            }),
            idle: state.idle,
        }
    }

    /// The teardown, once the unlock's tasks stopped: Offline, waiting at
    /// most [`OFFLINE_WRITE_WAIT`]. Nothing was published without a pool
    /// or a profile.
    pub(crate) async fn publish_offline(&self) {
        if self.profile_dht_info().is_none()
            || self.transport.as_ref().and_then(|t| t.records()).is_none()
        {
            return;
        }
        self.state.write().status = SessionStatus::Offline;
        match tokio::time::timeout(
            OFFLINE_WRITE_WAIT,
            rekindle_presence::publish_status(Arc::new(self.clone()), SessionStatus::Offline),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "failed to publish offline status"),
            Err(_) => tracing::info!("offline status still publishing; the lock continues"),
        }
    }

    fn elapsed_seconds(&self, game: &Game) -> u32 {
        u32::try_from(self.clock.now_ms().saturating_sub(game.started_at_ms) / 1000)
            .unwrap_or(u32::MAX)
    }

    /// Wake the publisher and tell every frontend.
    fn changed(&self) {
        self.wake.notify_one();
        let owner = self
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone());
        let Some(public_key) = owner else {
            return;
        };
        let view = self.view();
        let game = view
            .game
            .map_or(GameActivity::Idle, |game| GameActivity::Playing {
                game_name: game.game_name,
                game_id: Some(game.game_id),
                elapsed_seconds: Some(game.elapsed_seconds),
                server_address: game.server_address,
            });
        // Our own device sees our real status, Invisible included; peers
        // see the wire value (Invisible folds to "offline").
        let status = serde_json::to_value(view.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned));
        self.journal
            .publish(SubscriptionEvent::Presence(PresenceEvent::SelfChanged {
                public_key,
                snapshot: PresenceSnapshot {
                    status,
                    status_message: Some(view.status_message),
                    game: Some(game),
                    idle: Some(view.idle),
                },
            }));
    }

    /// Write one of our profile's subkeys through the session's pool.
    ///
    /// # Errors
    /// No profile yet, no pool, or the write failed.
    pub(crate) async fn write_profile(&self, subkey: u32, value: Vec<u8>) -> Result<(), String> {
        let Some(profile_key) = self.profile_dht_info() else {
            return Err("no profile yet: it is published when the network is up".into());
        };
        let pool = self
            .transport
            .as_ref()
            .and_then(|t| t.records())
            .ok_or("record pool not running")?;
        let outcome = rekindle_protocol::dht::profile::set_own_profile_subkey(
            pool.as_ref(),
            &profile_key,
            subkey,
            value,
        )
        .await
        .map_err(|e| format!("profile write: {e}"))?;
        if outcome.missed() {
            tracing::warn!(subkey, ?outcome, "profile write not stored at consensus");
        }
        Ok(())
    }
}

#[async_trait]
impl StatusPublisherDeps for OwnPresence {
    fn profile_dht_info(&self) -> Option<String> {
        self.session
            .read()
            .as_ref()
            .map(|s| s.identity.profile_dht_key.clone())
            .filter(|key| !key.is_empty())
    }

    async fn write_profile_status_subkey(
        &self,
        profile_key: &str,
        payload: Vec<u8>,
    ) -> Result<(), PresenceError> {
        let pool = self
            .transport
            .as_ref()
            .and_then(|t| t.records())
            .ok_or_else(|| PresenceError::Dht("record pool not running".into()))?;
        let outcome = rekindle_protocol::dht::profile::set_own_profile_status(
            pool.as_ref(),
            profile_key,
            payload,
        )
        .await
        .map_err(|e| PresenceError::Dht(e.to_string()))?;
        if outcome.missed() {
            return Err(PresenceError::Dht(format!(
                "status not stored ({outcome:?})"
            )));
        }
        Ok(())
    }

    fn presence_key(&self) -> Option<(String, rekindle_secrets::friend_presence::PresenceKey)> {
        self.session
            .read()
            .as_ref()
            .and_then(|s| Some((s.identity.public_key_hex.clone(), s.presence_key.clone()?)))
    }

    fn current_identity_status(&self) -> Option<SessionStatus> {
        Some(self.status())
    }

    fn now_ms(&self) -> i64 {
        self.clock.now_ms_i64()
    }
}

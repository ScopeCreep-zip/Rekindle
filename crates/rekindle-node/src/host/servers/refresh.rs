//! The Servers view's Refresh (plan E7.2 G4, decision 49): ask every listed
//! server what it is running, once, when the user asks (`ServersRefresh`).
//!
//! A query shows our IP address to the server it asks, a third party the
//! threat model counts as metadata leaving the app
//! (`docs/security/threat-model.md` § Scope), so the Host never queries on
//! its own: no timer, no view open, no event starts one; only this request
//! does. The protocol is the game database's (`GameEntry::server_query`,
//! A2S in `rekindle_game_detect::server_query`); the Host only orchestrates:
//! every row at once, at most [`CONCURRENCY`] in flight, each bounded by
//! [`QUERY_TIMEOUT`], each answer pushed through `ServersViewChanged`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rekindle_game_detect::server_query::{
    query_info, QueryError, ServerQuery, ServerQueryProtocol,
};
use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::servers::{
    RefreshAnswer, ServerInfo, ServerRefusal, ServerVerdict, ServersView,
};
use rekindle_types::game_server::ServerAddress;

use crate::daemon::dispatch::{state_error, DaemonContext};
use crate::daemon::DaemonState;

/// How long one server has to answer.
pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
/// Queries in flight at once.
const CONCURRENCY: usize = 16;

pub(crate) const NO_REPLY: &str = "No reply within 2 s";
pub(crate) const NO_QUERY: &str = "This game has no server query";
const NOT_AN_ADDRESS: &str = "Not a server address";
const UNREACHABLE: &str = "The server could not be reached";
const UNREADABLE: &str = "The server's reply could not be read";

/// A row's server: its game and `host:port`.
pub(crate) type InfoKey = (Option<u32>, String);

#[derive(Default)]
struct Results {
    /// Bumped when the session ends, so a refresh still running then never
    /// writes into the next session's view.
    generation: u64,
    info: HashMap<InfoKey, ServerInfo>,
}

/// The refresh's port and what it found this session (`ctx.servers`).
pub(crate) struct RefreshState {
    query: Arc<dyn ServerQuery>,
    running: AtomicBool,
    results: parking_lot::Mutex<Results>,
}

impl std::fmt::Debug for RefreshState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshState")
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl RefreshState {
    pub(crate) fn new(query: Arc<dyn ServerQuery>) -> Self {
        Self {
            query,
            running: AtomicBool::new(false),
            results: parking_lot::Mutex::new(Results::default()),
        }
    }

    /// What each server answered, as the view shows it.
    pub(crate) fn info(&self) -> HashMap<InfoKey, ServerInfo> {
        self.results.lock().info.clone()
    }

    /// `ServersView.refresh`.
    pub(crate) fn verdict(&self) -> ServerVerdict {
        if self.running.load(Ordering::Acquire) {
            ServerVerdict::refused(ServerRefusal::Refreshing)
        } else {
            ServerVerdict::Allowed
        }
    }

    fn record(&self, generation: u64, key: InfoKey, info: ServerInfo) {
        let mut results = self.results.lock();
        if results.generation == generation {
            results.info.insert(key, info);
        }
    }

    /// The session ended: forget every answer.
    pub(crate) fn clear(&self) {
        let mut results = self.results.lock();
        results.generation += 1;
        results.info.clear();
    }
}

/// `ServersRefresh`: mark every row `querying` (or `unanswered` when its
/// game has no query), push the view, and ask in the background. Answers
/// `started: false` while a refresh runs.
pub(crate) async fn start(ctx: &DaemonContext, state: DaemonState) -> IpcResponse {
    if !state.can_query() {
        return state_error(state, "query");
    }
    let refresh = &ctx.servers.refresh;
    if refresh.running.swap(true, Ordering::AcqRel) {
        return IpcResponse::ok(&RefreshAnswer { started: false });
    }
    let view = match super::compose(ctx).await {
        Ok(view) => view,
        Err(e) => {
            refresh.running.store(false, Ordering::Release);
            return e;
        }
    };
    let generation = refresh.results.lock().generation;
    let mut jobs = Vec::new();
    for (key, asked) in targets(&view) {
        let info = if asked.is_some() {
            ServerInfo::Querying
        } else {
            ServerInfo::Unanswered {
                reason: reason_without_query(&key).into(),
            }
        };
        refresh.record(generation, key.clone(), info);
        if let Some(server) = asked {
            jobs.push((key, server));
        }
    }
    let Some(ctx_arc) = ctx.this.upgrade() else {
        refresh.running.store(false, Ordering::Release);
        return IpcResponse::error(503, "the host is stopping");
    };
    super::push(ctx).await;
    tokio::spawn(run(ctx_arc, generation, jobs));
    IpcResponse::ok(&RefreshAnswer { started: true })
}

/// Each distinct server in the view, with the address to ask when its game
/// speaks a query protocol and the address parses.
fn targets(view: &ServersView) -> HashMap<InfoKey, Option<ServerAddress>> {
    let db = rekindle_game_detect::GameDatabase::shared();
    let mut out = HashMap::new();
    for row in view.categories.iter().flat_map(|c| &c.rows) {
        let protocol = row
            .game_id
            .and_then(|id| db.lookup_by_id(id))
            .and_then(|e| e.server_query);
        let asked = match protocol {
            Some(ServerQueryProtocol::A2s) => ServerAddress::parse(&row.address).ok(),
            None => None,
        };
        out.insert((row.game_id, row.address.clone()), asked);
    }
    out
}

fn reason_without_query(key: &InfoKey) -> &'static str {
    let db = rekindle_game_detect::GameDatabase::shared();
    let speaks = key
        .0
        .and_then(|id| db.lookup_by_id(id))
        .is_some_and(|e| e.server_query.is_some());
    if speaks {
        NOT_AN_ADDRESS
    } else {
        NO_QUERY
    }
}

async fn run(ctx: Arc<DaemonContext>, generation: u64, jobs: Vec<(InfoKey, ServerAddress)>) {
    let limit = Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let mut set = tokio::task::JoinSet::new();
    for (key, server) in jobs {
        let query = Arc::clone(&ctx.servers.refresh.query);
        let limit = Arc::clone(&limit);
        set.spawn(async move {
            let _permit = limit.acquire_owned().await;
            (key, ask(query.as_ref(), server.as_str()).await)
        });
    }
    while let Some(done) = set.join_next().await {
        if let Ok((key, info)) = done {
            ctx.servers.refresh.record(generation, key, info);
            ctx.servers.mark_stale();
        }
    }
    ctx.servers.refresh.running.store(false, Ordering::Release);
    ctx.servers.mark_stale();
}

/// One server's answer, or why there was none.
async fn ask(query: &dyn ServerQuery, server: &str) -> ServerInfo {
    let round = async {
        let mut channel = query.open(server).await?;
        query_info(channel.as_mut()).await
    };
    match tokio::time::timeout(QUERY_TIMEOUT, round).await {
        Err(_) => unanswered(NO_REPLY),
        Ok(Ok((info, rtt))) => ServerInfo::Answered {
            name: info.name,
            map: info.map,
            players: u32::from(info.players),
            max_players: u32::from(info.max_players),
            ping_ms: u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX),
        },
        Ok(Err(QueryError::Io(_))) => unanswered(UNREACHABLE),
        Ok(Err(_)) => unanswered(UNREADABLE),
    }
}

fn unanswered(reason: &str) -> ServerInfo {
    ServerInfo::Unanswered {
        reason: reason.into(),
    }
}

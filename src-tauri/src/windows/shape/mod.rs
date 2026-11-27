//! One allocation-driven controller per framed window. The validated Host
//! frames, actual native allocation and backing scale determine the shape.
//! No webview message can supply a shape, margins or cursor-ignore state.

#[cfg(test)]
mod allocation_goldens;
mod geometry;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", test))]
mod logical;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "linux"))]
mod native;
#[cfg(any(target_os = "linux", test))]
mod settle;
mod sizing;
mod skin;
#[cfg(windows)]
mod win32_region;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_lifecycle::SessionScope;
use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_library::{SkinApplied, SkinApplyFailure};
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};
use tokio::sync::watch;

use crate::event_dispatch::WebviewEvent;
use crate::event_router::Audience;
use crate::state::SharedState;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

pub use geometry::{ShapeRect, Silhouette};
pub use skin::{frame_of, frame_of_label, refresh};

/// The frames and the stylesheet obtained from the same Host content key.
struct Generation {
    serial: u64,
    frames: skin::SkinFrameMap,
    skin: Option<SkinApplied>,
}

struct Shaped {
    frame: SkinWindow,
    presented: watch::Sender<Option<SkinApplied>>,
    applied: Mutex<Option<Arc<Generation>>>,
    unfitted: AtomicU64,
    tasks: Arc<SessionScope>,
    #[cfg(not(target_os = "linux"))]
    native: Mutex<native::State>,
    #[cfg(not(target_os = "linux"))]
    queued: AtomicBool,
    #[cfg(target_os = "macos")]
    current: Arc<Mutex<Option<Silhouette>>>,
    #[cfg(target_os = "macos")]
    ignoring: Arc<AtomicBool>,
}

impl Shaped {
    fn spawn(&self, name: &'static str, task: impl Future<Output = ()> + Send + 'static) {
        // Native callbacks can arrive on a UI thread outside the runtime.
        let runtime = tauri::async_runtime::handle();
        let _entered = runtime.inner().enter();
        self.tasks.spawn_or_drop(name, task);
    }
}

struct ShapeRegistry {
    windows: Mutex<HashMap<String, Arc<Shaped>>>,
    generation: Mutex<Arc<Generation>>,
    requested: AtomicBool,
    fetch: AtomicU64,
}

impl Default for ShapeRegistry {
    fn default() -> Self {
        Self {
            windows: Mutex::default(),
            generation: Mutex::new(Arc::new(Generation {
                serial: 0,
                frames: skin::SkinFrameMap::new(),
                skin: None,
            })),
            requested: AtomicBool::new(false),
            fetch: AtomicU64::new(0),
        }
    }
}

fn registry(app: &AppHandle) -> tauri::State<'_, ShapeRegistry> {
    if app.try_state::<ShapeRegistry>().is_none() {
        app.manage(ShapeRegistry::default());
    }
    app.state::<ShapeRegistry>()
}

fn desired(app: &AppHandle) -> Arc<Generation> {
    Arc::clone(&registry(app).generation.lock())
}

fn is_latest(app: &AppHandle, generation: &Generation) -> bool {
    registry(app).fetch.load(Ordering::SeqCst) == generation.serial
}

fn is_current(window: &WebviewWindow, shaped: &Arc<Shaped>) -> bool {
    registry(window.app_handle())
        .windows
        .lock()
        .get(window.label())
        .is_some_and(|live| Arc::ptr_eq(live, shaped))
}

fn sizing(generation: &Generation, frame: SkinWindow) -> sizing::Sizing {
    sizing::Sizing::for_frame(
        frame,
        generation.frames.get(&frame).map(|(layout, _)| layout),
    )
}

/// Prefer the desired validated frames. A constrained canvas can keep an
/// earlier generation until it can accommodate the new one.
fn fitting(
    window: &WebviewWindow,
    shaped: &Shaped,
    desired: &Arc<Generation>,
    canvas: (f64, f64),
) -> Option<Arc<Generation>> {
    let fits = |generation: &Generation| {
        !generation.frames.contains_key(&shaped.frame)
            || sizing(generation, shaped.frame).fits(canvas)
    };
    if is_latest(window.app_handle(), desired) && fits(desired) {
        return Some(Arc::clone(desired));
    }
    shaped
        .applied
        .lock()
        .as_ref()
        .filter(|applied| fits(applied))
        .map(Arc::clone)
}

/// Use validated sizing at construction when it is already available.
/// Otherwise start with the window kind's native defaults.
pub(super) fn initial_sizing(
    app: &AppHandle,
    label: &str,
) -> Option<((f64, f64), tauri::WindowSizeConstraints)> {
    let frame = frame_of_label(label)?;
    let generation = desired(app);
    let policy = if is_latest(app, &generation) {
        sizing(&generation, frame)
    } else {
        sizing::Sizing::for_frame(frame, None)
    };
    Some((policy.wanted, policy.constraints()))
}

/// Called only by a successful native allocation pass. A newer generation
/// or a new window under the same label invalidates delayed work.
fn publish(
    window: &WebviewWindow,
    shaped: &Arc<Shaped>,
    wanted: &Arc<Generation>,
    fitted: Option<Arc<Generation>>,
) {
    let app = window.app_handle();
    let state = registry(app);
    let windows = state.windows.lock();
    let selected = state.generation.lock();
    if !windows
        .get(window.label())
        .is_some_and(|live| Arc::ptr_eq(live, shaped))
        || !Arc::ptr_eq(&selected, wanted)
    {
        return;
    }
    let skin = match fitted {
        Some(applied) => {
            let mut previous = shaped.applied.lock();
            let current = Arc::ptr_eq(wanted, &applied)
                && state.fetch.load(Ordering::SeqCst) == applied.serial;
            let retained = previous
                .as_ref()
                .is_some_and(|saved| Arc::ptr_eq(saved, &applied));
            if !current && !retained {
                return;
            }
            let skin = applied.skin.clone();
            *previous = Some(applied);
            skin
        }
        None => None,
    };
    if *shaped.presented.borrow() == skin {
        return;
    }
    let event = match &skin {
        Some(skin) => {
            WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinChanged {
                skin: skin.clone(),
            }))
        }
        None => WebviewEvent::SkinFrameBase {
            window: window.label().to_owned(),
        },
    };
    // This presentation may follow a newer Host cursor. Its generation has
    // already been checked above, so the normal stream must not discard it.
    if let (Some(state), Some(envelope)) = (app.try_state::<SharedState>(), event.envelope(None)) {
        state
            .event_router
            .deliver(&envelope, &Audience::labels([window.label().to_owned()]));
    }
    shaped.presented.send_replace(skin);
}

fn report_fit(
    window: &WebviewWindow,
    shaped: &Shaped,
    wanted: &Generation,
    fits: bool,
    canvas: (f64, f64),
) {
    if fits {
        shaped.unfitted.store(0, Ordering::Release);
        return;
    }
    if !is_latest(window.app_handle(), wanted) {
        return;
    }
    let Some((layout, _)) = wanted.frames.get(&shaped.frame) else {
        return;
    };
    let Some((width, height)) = rekindle_skin::shape::min_window(layout) else {
        return;
    };
    let Some(skin) = &wanted.skin else { return };
    if shaped.unfitted.swap(wanted.serial, Ordering::AcqRel) == wanted.serial {
        return;
    }
    let label = window.label().to_owned();
    tracing::warn!(%label, skin = %skin.id, ?canvas, width, height, "window_skin_too_small");
    let Some(state) = window
        .app_handle()
        .try_state::<SharedState>()
        .map(|state| Arc::clone(&state))
    else {
        return;
    };
    let request = IpcRequest::SkinApplyFailed {
        window: label,
        skin: skin.id.clone(),
        cause: SkinApplyFailure::WindowTooSmall { width, height },
    };
    shaped.spawn("report constrained skin window", async move {
        if let Err(error) = crate::host_call::call::<()>(&state, request).await {
            tracing::warn!(?error, "window skin size could not be reported");
        }
    });
}

/// Bootstrap a framed webview from its native presentation, including
/// reloads during a pending switch. Unframed windows query the Host directly.
pub async fn presented_skin(window: &WebviewWindow) -> Option<SkinApplied> {
    let mut presented = registry(window.app_handle())
        .windows
        .lock()
        .get(window.label())?
        .presented
        .subscribe();
    let skin = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        presented.wait_for(Option::is_some),
    )
    .await
    .ok()?
    .ok()?
    .clone();
    skin
}

/// Attach before the first show. GTK owns the realized root and calls its
/// allocation handler. Other desktops schedule one native UI turn per burst.
pub fn attach(window: &WebviewWindow, frame: Option<SkinWindow>) {
    let Some(frame) = frame else { return };
    let scoped = Arc::new(Shaped {
        frame,
        presented: watch::channel(None).0,
        applied: Mutex::default(),
        unfitted: AtomicU64::new(0),
        tasks: SessionScope::new(
            "window shape",
            Arc::new(|task| tracing::error!(task, "window shape task panicked")),
        ),
        #[cfg(not(target_os = "linux"))]
        native: Mutex::default(),
        #[cfg(not(target_os = "linux"))]
        queued: AtomicBool::new(false),
        #[cfg(target_os = "macos")]
        current: Arc::new(Mutex::new(None)),
        #[cfg(target_os = "macos")]
        ignoring: Arc::new(AtomicBool::new(false)),
    });
    let app = window.app_handle();
    registry(app)
        .windows
        .lock()
        .insert(window.label().to_owned(), Arc::clone(&scoped));
    #[cfg(target_os = "macos")]
    macos::start(
        window.clone(),
        Arc::clone(&scoped.current),
        Arc::clone(&scoped.ignoring),
        scoped.tasks.token(),
    );

    let target = window.clone();
    let watched = Arc::clone(&scoped);
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            forget(&target, &watched);
        }
        #[cfg(not(target_os = "linux"))]
        if matches!(
            event,
            WindowEvent::Resized(_)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Focused(_)
        ) {
            native::queue(&target, &watched);
        }
    });
    #[cfg(target_os = "linux")]
    linux::attach(window, &scoped);
    #[cfg(not(target_os = "linux"))]
    native::queue(window, &scoped);
    if !registry(app).requested.swap(true, Ordering::AcqRel) {
        refresh(app, None);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn revealed(window: &WebviewWindow) {
    let scoped = registry(window.app_handle())
        .windows
        .lock()
        .get(window.label())
        .cloned();
    if let Some(scoped) = scoped {
        native::queue(window, &scoped);
    }
}

fn forget(window: &WebviewWindow, shaped: &Arc<Shaped>) {
    let state = registry(window.app_handle());
    let mut windows = state.windows.lock();
    if windows
        .get(window.label())
        .is_some_and(|live| Arc::ptr_eq(live, shaped))
    {
        windows.remove(window.label());
    }
    shaped.tasks.token().cancel();
}

fn begin_fetch(app: &AppHandle) -> u64 {
    let state = registry(app);
    let _presentation = state.generation.lock();
    state.requested.store(true, Ordering::Release);
    state.fetch.fetch_add(1, Ordering::SeqCst) + 1
}

fn set_frames(app: &AppHandle, serial: u64, frames: skin::SkinFrameMap, skin: SkinApplied) {
    let state = registry(app);
    {
        let mut desired = state.generation.lock();
        if state.fetch.load(Ordering::SeqCst) != serial {
            return;
        }
        *desired = Arc::new(Generation {
            serial,
            frames,
            skin: Some(skin),
        });
    }
    let windows = state.windows.lock().clone();
    for (label, shaped) in windows {
        if let Some(window) = app.get_webview_window(&label) {
            #[cfg(target_os = "linux")]
            linux::queue(&window, &shaped);
            #[cfg(not(target_os = "linux"))]
            native::queue(&window, &shaped);
        }
    }
}

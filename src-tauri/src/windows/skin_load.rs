//! Native ownership of the skin-load deadline and the window it reveals.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use rekindle_ipc::protocol::IpcRequest;
use rekindle_lifecycle::SessionScope;
use rekindle_types::dto::skin_library::{SkinApplied, SkinApplyFailure};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::skin_reveal::{Show, SkinRevealEffect, SkinRevealGate, LOAD_LIMIT};
use crate::state::SharedState;

#[derive(Clone, Copy)]
enum Opening {
    Waiting,
    Skinned,
    Failed,
}

// A document restart resets its local serial. Native attempt IDs are
// independent of a document and survive window label reuse.
static NEXT_APPLY_ID: AtomicU64 = AtomicU64::new(1);

struct Apply {
    skin: String,
    href: String,
    end: Instant,
    started: Option<(String, Channel<&'static str>)>,
    stop: CancellationToken,
}

impl Drop for Apply {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

struct Load {
    state: Mutex<State>,
    tasks: Arc<SessionScope>,
}

impl Load {
    fn new() -> Self {
        Self {
            state: Mutex::default(),
            tasks: SessionScope::new(
                "skin window load",
                Arc::new(|task| tracing::error!(task, "skin window task panicked")),
            ),
        }
    }

    fn spawn(&self, name: &'static str, task: impl Future<Output = ()> + Send + 'static) {
        super::spawn_window_task(&self.tasks, name, task);
    }
}

struct State {
    gate: SkinRevealGate,
    apply: Option<Apply>,
    timed_out: Option<String>,
    serial: u32,
    staged: bool,
    opening: watch::Sender<Opening>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            gate: SkinRevealGate::default(),
            apply: None,
            timed_out: None,
            serial: 0,
            staged: false,
            opening: watch::channel(Opening::Waiting).0,
        }
    }
}

impl State {
    fn new_document(&mut self) {
        self.apply = None;
        self.timed_out = None;
        self.serial = 0;
        self.gate.new_document();
        self.opening = watch::channel(Opening::Waiting).0;
    }

    fn expects_delivery(&self, skin: &str, href: &str) -> bool {
        !self.gate.failed()
            && self.apply.as_ref().map_or_else(
                || !self.gate.committed_to(href),
                |apply| apply.skin != skin || apply.href != href,
            )
    }

    fn deliver(
        &mut self,
        skin: String,
        href: String,
        now: Instant,
    ) -> (Instant, CancellationToken) {
        let stop = CancellationToken::new();
        let end = now + LOAD_LIMIT;
        self.timed_out = None;
        self.apply = Some(Apply {
            skin,
            href,
            end,
            started: None,
            stop: stop.clone(),
        });
        (end, stop)
    }

    fn begin_apply(
        &mut self,
        serial: u32,
        skin: &str,
        href: &str,
        expired: Channel<&'static str>,
        now: Instant,
    ) -> Option<String> {
        if self.gate.failed() || self.timed_out.as_deref() == Some(href) {
            let _ = expired.send(SkinApplyFailure::LOAD_TIMEOUT_MESSAGE);
            return None;
        }
        let apply = self.apply.as_mut()?;
        if serial <= self.serial || skin != apply.skin || href != apply.href || now >= apply.end {
            return None;
        }
        let id = NEXT_APPLY_ID.fetch_add(1, Ordering::Relaxed).to_string();
        self.serial = serial;
        apply.started = Some((id.clone(), expired));
        Some(id)
    }

    fn expire_apply(&mut self, now: Instant) -> Option<Apply> {
        let apply = self.apply.take_if(|apply| now >= apply.end)?;
        self.timed_out = Some(apply.href.clone());
        Some(apply)
    }

    /// Complete only the native attempt that the document started.
    fn finish_apply(&mut self, attempt_id: &str, href: Option<String>, now: Instant) -> bool {
        if self
            .apply
            .take_if(|apply| {
                now < apply.end
                    && href.as_ref().is_none_or(|href| href == &apply.href)
                    && apply
                        .started
                        .as_ref()
                        .is_some_and(|(id, _)| id == attempt_id)
            })
            .is_none()
        {
            return false;
        }
        if let Some(href) = href {
            self.gate.commit(href, now);
            return true;
        }
        false
    }
}

#[derive(Default)]
struct Loads(Mutex<HashMap<String, Arc<Load>>>);

fn registry(app: &AppHandle) -> tauri::State<'_, Loads> {
    if app.try_state::<Loads>().is_none() {
        app.manage(Loads::default());
    }
    app.state::<Loads>()
}

fn load(window: &WebviewWindow) -> Arc<Load> {
    Arc::clone(
        registry(window.app_handle())
            .0
            .lock()
            .entry(window.label().to_owned())
            .or_insert_with(|| Arc::new(Load::new())),
    )
}

fn forget(window: &WebviewWindow, load: &Arc<Load>) {
    let registry = registry(window.app_handle());
    let mut windows = registry.0.lock();
    if windows
        .get(window.label())
        .is_some_and(|live| Arc::ptr_eq(live, load))
    {
        windows.remove(window.label());
    }
    load.state.lock().apply = None;
    load.tasks.token().cancel();
}

fn current(window: &WebviewWindow, load: &Arc<Load>) -> bool {
    registry(window.app_handle())
        .0
        .lock()
        .get(window.label())
        .is_some_and(|live| Arc::ptr_eq(live, load))
}

pub(super) fn prepare(window: &WebviewWindow) {
    let load = load(window);
    let target = window.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            forget(&target, &load);
        }
    });
}

pub(super) fn framed(window: &WebviewWindow) -> bool {
    super::frame_of_label(window.label()).is_some()
}

/// The previous document's commit and applies cannot survive a reload.
pub(super) fn page_started(window: &WebviewWindow) {
    let load = load(window);
    let mut state = load.state.lock();
    if state.gate.failed() {
        // The error window stays in its terminal state until the user closes it.
        return;
    }
    state.new_document();
    drop(state);
    if framed(window) && window.is_visible().unwrap_or(false) {
        request(window, Show::Raise);
    }
}

/// The load from the last successfully configured native allocation.
pub(super) fn allocated(window: &WebviewWindow, href: Option<&str>) {
    let live = registry(window.app_handle())
        .0
        .lock()
        .get(window.label())
        .cloned();
    if let Some(load) = live {
        let mut state = load.state.lock();
        state.gate.allocate(href, Instant::now());
        let pending = state.gate.pending();
        drop(state);
        if pending {
            enqueue(window, &load);
        }
    }
}

pub(super) fn request(window: &WebviewWindow, show: Show) {
    let load = load(window);
    if let Some(end) = load.state.lock().gate.request(Instant::now(), show) {
        let target = window.clone();
        let pending = Arc::downgrade(&load);
        let stop = load.tasks.token();
        load.spawn("wait for skin window deadline", async move {
            tokio::select! {
                () = stop.cancelled() => {}
                () = tokio::time::sleep_until(end.into()) => {
                    if let Some(load) = pending.upgrade() {
                        enqueue(&target, &load);
                    }
                }
            }
        });
    }
    enqueue(window, &load);
}

fn enqueue(window: &WebviewWindow, load: &Arc<Load>) {
    let window = window.clone();
    let live = Arc::clone(load);
    // Tauri executes run_on_main_thread inline on the UI thread. Post a
    // fresh turn so GTK can finish its allocation callback before showing.
    load.spawn("queue skin reveal", async move {
        let target = window.clone();
        log_window(
            window.run_on_main_thread(move || {
                if current(&target, &live) {
                    update(&target, &live);
                }
            }),
            &window,
            "queue skin reveal",
        );
    });
}

fn update(window: &WebviewWindow, load: &Load) {
    let mut state = load.state.lock();
    let needs_staging = state.gate.pending() && !state.staged && hidden(window);
    #[cfg(target_os = "linux")]
    if needs_staging {
        state.gate.allocate(None, Instant::now());
    }
    let effect = state.gate.poll(Instant::now());
    if effect == SkinRevealEffect::Wait {
        if needs_staging {
            state.staged = true;
            drop(state);
            prepare_allocation(window);
        }
        return;
    }

    let staged = std::mem::take(&mut state.staged);
    let report = match effect {
        SkinRevealEffect::Skinned(_) => {
            state.opening.send_replace(Opening::Skinned);
            None
        }
        SkinRevealEffect::RevealAndReportTimeout(_) => {
            state.opening.send_replace(Opening::Failed);
            let skin = state.apply.take().map(|apply| apply.skin.clone());
            // The live apply may already have reported the same timeout.
            state.timed_out.is_none().then_some(skin)
        }
        SkinRevealEffect::Wait | SkinRevealEffect::Failed(_) => None,
    };
    drop(state);
    match effect {
        SkinRevealEffect::Wait => unreachable!(),
        SkinRevealEffect::Skinned(show) | SkinRevealEffect::Failed(show) => {
            expose(window, show, staged);
        }
        SkinRevealEffect::RevealAndReportTimeout(show) => {
            // The native error has its own close control even if the page
            // is suspended or its entry module never ran.
            expose(window, show, staged);
            if let Some(skin) = report {
                report_timeout(window, load, skin);
            }
            show_timeout_dialog(window);
        }
    }
}

// rfd's GTK message dialog does not use its raw parent handle. Use the
// already mapped GTK toplevel so Wayland can place the dialog over it.
#[cfg(target_os = "linux")]
fn show_timeout_dialog(window: &WebviewWindow) {
    use gtk::prelude::{DialogExt, WidgetExt};

    let parent = match window.gtk_window() {
        Ok(parent) => parent,
        Err(error) => {
            tracing::warn!(%error, label = window.label(), "skin error dialog parent unavailable");
            return;
        }
    };
    let dialog = gtk::MessageDialog::builder()
        .transient_for(&parent)
        .destroy_with_parent(true)
        .modal(true)
        .title("Skin load failed")
        .message_type(gtk::MessageType::Error)
        .text(SkinApplyFailure::LOAD_TIMEOUT_MESSAGE)
        .build();
    dialog.add_button("Close window", gtk::ResponseType::Close);
    dialog.set_default_response(gtk::ResponseType::Close);
    let target = window.clone();
    dialog.connect_response(move |dialog, _| {
        dialog.hide();
        log_window(target.destroy(), &target, "close failed skin window");
    });
    dialog.show_all();
}

#[cfg(not(target_os = "linux"))]
fn show_timeout_dialog(window: &WebviewWindow) {
    let target = window.clone();
    window
        .dialog()
        .message(SkinApplyFailure::LOAD_TIMEOUT_MESSAGE)
        .title("Skin load failed")
        .kind(MessageDialogKind::Error)
        .parent(window)
        .buttons(MessageDialogButtons::OkCustom("Close window".to_owned()))
        .show(move |_| log_window(target.destroy(), &target, "close failed skin window"));
}

pub(crate) fn is_revealed(window: &WebviewWindow) -> bool {
    let staged = registry(window.app_handle())
        .0
        .lock()
        .get(window.label())
        .is_some_and(|load| load.state.lock().staged);
    !staged && window.is_visible().unwrap_or(false)
}

fn hidden(window: &WebviewWindow) -> bool {
    !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false)
}

// GTK does not allocate a toplevel until it maps. Map it with neither
// pixels nor focus, with the same undecorated ordering as first_show.
#[cfg(target_os = "linux")]
fn prepare_allocation(window: &WebviewWindow) {
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    match window.gtk_window() {
        Ok(widget) => {
            widget.set_opacity(0.0);
            widget.set_sensitive(false);
            widget.set_accept_focus(false);
            widget.set_skip_taskbar_hint(true);
            log_window(
                window.show(),
                window,
                "map transparent GTK window for allocation",
            );
            log_window(
                window.unminimize(),
                window,
                "restore transparent GTK window for allocation",
            );
        }
        Err(error) => tracing::warn!(%error, label = window.label(), "GTK window unavailable"),
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare_allocation(window: &WebviewWindow) {
    super::shape::revealed(window);
}

fn expose(window: &WebviewWindow, show: Show, staged: bool) {
    #[cfg(target_os = "linux")]
    if staged {
        use gtk::prelude::{GtkWindowExt, WidgetExt};
        if let Ok(widget) = window.gtk_window() {
            widget.set_sensitive(true);
            widget.set_accept_focus(true);
            widget.set_skip_taskbar_hint(false);
            widget.set_opacity(1.0);
        }
    }
    // A staged GTK window first mapped without focus.
    if staged || show == Show::Raise {
        super::reveal::reveal(window);
    } else {
        log_window(window.show(), window, "first skinned show");
    }
}

fn log_window(result: tauri::Result<()>, window: &WebviewWindow, action: &str) {
    if let Err(error) = result {
        tracing::warn!(%error, window = window.label(), action, "skin window operation failed");
    }
}

fn report_timeout(window: &WebviewWindow, load: &Load, skin: Option<String>) {
    let Some(state) = window
        .app_handle()
        .try_state::<SharedState>()
        .map(|s| Arc::clone(&s))
    else {
        return;
    };
    let window = window.label().to_owned();
    load.spawn("report skin load timeout", async move {
        let request = IpcRequest::SkinApplyFailed {
            window,
            skin,
            cause: SkinApplyFailure::load_timeout(),
        };
        // The native dialog shows the same words (`LOAD_TIMEOUT_MESSAGE`).
        if let Err(error) = crate::host_call::call::<String>(&state, request).await {
            tracing::warn!(?error, "skin load timeout could not be reported");
        }
    });
}

/// The document observes the same deadline which releases its native window:
/// this resolves when this document's reveal gate fails (its deadline expired),
/// with the failure's message, and never for a skinned reveal.
pub(crate) async fn wait_open(
    window: &WebviewWindow,
) -> Result<Option<&'static str>, &'static str> {
    if !framed(window) {
        return Ok(None);
    }
    let mut opening = load(window).state.lock().opening.subscribe();
    let status = opening
        .wait_for(|status| matches!(status, Opening::Failed))
        .await
        .map_err(|_| "The skin load document was replaced")?;
    Ok(matches!(*status, Opening::Failed).then_some(SkinApplyFailure::LOAD_TIMEOUT_MESSAGE))
}

/// Accept a live Host switch for a window, even before a framed window's
/// native allocation can release the matching event to its document.
pub(crate) fn delivered(window: &WebviewWindow, skin: &SkinApplied) {
    let Some(load) = registry(window.app_handle())
        .0
        .lock()
        .get(window.label())
        .cloned()
    else {
        return;
    };
    let mut state = load.state.lock();
    // The snapshot or a replay can already have started or committed this
    // sheet. A duplicate live event may be dropped by the document's cursor.
    if state.expects_delivery(&skin.id, &skin.tokens_href) {
        let (end, stop) = state.deliver(skin.id.clone(), skin.tokens_href.clone(), Instant::now());
        watch_apply(window, &load, end, stop);
    }
}

/// The settled native allocation reported WindowTooSmall for this skin.
pub(super) fn refused(window: &WebviewWindow, skin: &SkinApplied) {
    let load = load(window);
    let mut state = load.state.lock();
    state.apply.take_if(|apply| apply.href == skin.tokens_href);
}

/// Claim the delivered switch. A document's initial snapshot also needs a
/// deadline when there is no live delivery for it.
pub(crate) fn start_apply(
    window: &WebviewWindow,
    serial: u32,
    skin: &str,
    href: &str,
    expired: Channel<&'static str>,
) -> Option<String> {
    let load = load(window);
    let mut state = load.state.lock();
    let now = Instant::now();
    if state.apply.is_none()
        && state.timed_out.as_deref() != Some(href)
        && !state.gate.failed()
        && serial > state.serial
    {
        let (end, stop) = state.deliver(skin.to_owned(), href.to_owned(), now);
        watch_apply(window, &load, end, stop);
    }
    state.begin_apply(serial, skin, href, expired, now)
}

fn watch_apply(window: &WebviewWindow, load: &Arc<Load>, end: Instant, stop: CancellationToken) {
    let target = window.clone();
    let active = Arc::clone(load);
    load.spawn("wait for skin apply deadline", async move {
        tokio::select! {
            () = stop.cancelled() => {}
            () = tokio::time::sleep_until(end.into()) => {
                let Some(apply) = expired_apply(&active, &stop) else { return };
                if let Some((_, expired)) = &apply.started {
                    let _ = expired.send(SkinApplyFailure::LOAD_TIMEOUT_MESSAGE);
                }
                report_timeout(&target, &active, Some(apply.skin.clone()));
            }
        }
    });
}

fn expired_apply(load: &Load, stop: &CancellationToken) -> Option<Apply> {
    let mut state = load.state.lock();
    if stop.is_cancelled() || state.gate.failed() {
        return None;
    }
    state.expire_apply(Instant::now())
}

pub(crate) fn finish_apply(window: &WebviewWindow, attempt_id: &str, href: Option<String>) {
    let load = load(window);
    let committed = load
        .state
        .lock()
        .finish_apply(attempt_id, href, Instant::now());
    if committed && framed(window) {
        enqueue(window, &load);
    }
}

#[cfg(test)]
#[path = "skin_load_tests.rs"]
mod tests;

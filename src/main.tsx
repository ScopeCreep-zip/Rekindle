/* @refresh reload */
import { render } from "solid-js/web";
import { createSignal, onMount, Match, Switch, lazy, Suspense } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "./ipc/commands";
import "./styles/global.css";
import AnnounceRegion from "./components/common/AnnounceRegion";
import CallController from "./components/voice/CallController";

/// Phase 10 — `localStorage` key holding the most recent journal cursor
/// this window has observed. Updated on every `cursor-tick` event AND
/// by `resumeEvents` after a successful backlog replay.
const LAST_CURSOR_KEY = "rekindle.lastEventCursor";

/// Race-safe cursor advance. Live `cursor-tick` events and replay-path
/// cursor-ticks can land in any order — using `max(stored, new)` means
/// out-of-order ticks never regress the persisted cursor. Wrapped in
/// try/catch because localStorage can be disabled (private mode) or
/// quota-exceeded; either way live events still flow.
function advanceCursor(next: number) {
  try {
    const current = Number(localStorage.getItem(LAST_CURSOR_KEY) ?? "0");
    if (next > current) {
      localStorage.setItem(LAST_CURSOR_KEY, String(next));
    }
  } catch {
    // localStorage unavailable — drop the update silently.
  }
}

/// Replay backlog from the backend journal before live listeners install.
/// Best-effort: any failure is logged + swallowed so it can't block boot.
///
/// IMPORTANT: the journal is in-memory only. On `kill -9` followed by
/// restart, the journal is empty and `event_resume` returns `0` — true
/// cold-start recovery for DMs/community messages comes from the SQLite
/// message-history APIs, not this path. This resume path is for soft
/// stalls: page reload during dev, brief IPC pauses, hot-reload across
/// the Tauri ↔ webview bridge.
///
/// The backend handles the actual re-emit — scoped to THIS webview via
/// `EventTarget::WebviewWindow`. The cursor advances organically through
/// the cursor-tick listener for each replayed entry; no manual
/// localStorage update needed here.
async function resumeEvents() {
  // E2E mode runs in a regular browser with no Tauri backend — skip.
  if (import.meta.env.VITE_E2E === "true") return;
  let cursor = 0;
  try {
    const raw = localStorage.getItem(LAST_CURSOR_KEY);
    const parsed = Number(raw ?? "0");
    // Defensive: localStorage values are strings and could be corrupted
    // (e.g. set by a stale schema, browser bug, or user tampering). A
    // NaN/Infinity cursor would serialize over Tauri IPC as `null` and
    // fail u64 deserialization on the backend; fall back to 0 so we
    // ask for the full backlog instead of erroring out of resume.
    cursor = Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
  } catch {
    // localStorage disabled (private mode, etc.) — proceed with cursor=0.
  }
  try {
    const count = await commands.eventResume(cursor);
    if (count > 0) {
      // Dev-mode signal that resume actually delivered something.
      console.debug(`event_resume replayed ${count} entries since cursor ${cursor}`);
    }
  } catch (err) {
    console.warn("event_resume failed; live events will still flow", err);
  }
}

/// Persist the latest cursor on every live emit. The backend's
/// `emit_journaled` helper fires `cursor-tick` alongside the original
/// payload event, so the frontend only needs ONE listener regardless of
/// how many payload channels exist.
function installCursorTickListener() {
  if (import.meta.env.VITE_E2E === "true") return;
  void listen<{ cursor: number }>("cursor-tick", (e) => {
    if (typeof e.payload?.cursor === "number") {
      advanceCursor(e.payload.cursor);
    }
  });
}

// Lazy-load window components so each webview only compiles
// the module tree it actually renders (login → 1 tree, not 6).
const LoginWindow = lazy(() => import("./windows/LoginWindow"));
const BuddyListWindow = lazy(() => import("./windows/BuddyListWindow"));
const ChatWindow = lazy(() => import("./windows/ChatWindow"));
const DmWindow = lazy(() => import("./windows/DmWindow"));
const CommunityWindow = lazy(() => import("./windows/CommunityWindow"));
const SettingsWindow = lazy(() => import("./windows/SettingsWindow"));
const ProfileWindow = lazy(() => import("./windows/ProfileWindow"));
const CallWindow = lazy(() => import("./windows/CallWindow"));

function App() {
  const [route, setRoute] = createSignal(window.location.pathname);

  onMount(() => {
    setRoute(window.location.pathname);
    // Phase 10 — install cursor-tick listener BEFORE resuming so the
    // replayed emits (which fire cursor-ticks of their own) also update
    // the persisted cursor. Then resume the backlog.
    installCursorTickListener();
    void resumeEvents();
  });

  return (
    <>
      {/* Architecture §32 a11y — module-level announce regions for
       * transient status messages. Mounted once per webview so the
       * `announce(...)` helper from AnnounceRegion.tsx always has a
       * live region to write into, regardless of which window is
       * currently rendered by the route Switch. */}
      <AnnounceRegion />
      {/* Wave 12 W12.1 — global call/notification subscription host so
       *  incoming calls ring + show modal + persist notifications in any
       *  webview, not only the BuddyListWindow context. */}
      <CallController />
      <Suspense>
        <Switch fallback={<LoginWindow />}>
          <Match when={route() === "/login"}>
            <LoginWindow />
          </Match>
          <Match when={route() === "/buddy-list"}>
            <BuddyListWindow />
          </Match>
          <Match when={route().startsWith("/chat")}>
            <ChatWindow />
          </Match>
          <Match when={route().startsWith("/dm")}>
            <DmWindow />
          </Match>
          <Match when={route().startsWith("/community")}>
            <CommunityWindow />
          </Match>
          <Match when={route() === "/settings"}>
            <SettingsWindow />
          </Match>
          <Match when={route().startsWith("/profile")}>
            <ProfileWindow />
          </Match>
          <Match when={route().startsWith("/call")}>
            <CallWindow />
          </Match>
        </Switch>
      </Suspense>
    </>
  );
}

render(() => <App />, document.getElementById("root")!);

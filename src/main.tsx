/* @refresh reload */
import { render } from "solid-js/web";
import { createSignal, onMount, Match, Switch, lazy, Suspense } from "solid-js";
import "./styles/global.css";
import AnnounceRegion from "./components/common/AnnounceRegion";
import CallController from "./components/voice/CallController";

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

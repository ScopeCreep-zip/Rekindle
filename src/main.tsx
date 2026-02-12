/* @refresh reload */
import { render } from "solid-js/web";
import { createSignal, onMount, Match, Switch, lazy, Suspense } from "solid-js";
import "./styles/global.css";

// Lazy-load window components so each webview only compiles
// the module tree it actually renders (login → 1 tree, not 6).
const LoginWindow = lazy(() => import("./windows/LoginWindow"));
const BuddyListWindow = lazy(() => import("./windows/BuddyListWindow"));
const ChatWindow = lazy(() => import("./windows/ChatWindow"));
const CommunityWindow = lazy(() => import("./windows/CommunityWindow"));
const SettingsWindow = lazy(() => import("./windows/SettingsWindow"));
const ProfileWindow = lazy(() => import("./windows/ProfileWindow"));

function App() {
  const [route, setRoute] = createSignal(window.location.pathname);

  onMount(() => {
    setRoute(window.location.pathname);
  });

  return (
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
        <Match when={route().startsWith("/community")}>
          <CommunityWindow />
        </Match>
        <Match when={route() === "/settings"}>
          <SettingsWindow />
        </Match>
        <Match when={route().startsWith("/profile")}>
          <ProfileWindow />
        </Match>
      </Switch>
    </Suspense>
  );
}

render(() => <App />, document.getElementById("root")!);

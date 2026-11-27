import { type Component, Show } from "solid-js";
import SkinTreeWindow from "../components/skin-tree/SkinTreeWindow";
import { SlotMount, SlotProvider } from "../components/skin-tree/slots";
import Titlebar from "../components/titlebar/Titlebar";
import CallPipelineHost from "../components/voice/call_stage/CallPipelineHost";
import { skinShell } from "../hooks/useSkin";
import { formatTimestamp } from "../utils/formatting";
import { removalText } from "../utils/removal_text";
import { setWindowKind } from "../utils/window_kind";
import CommunityMainPane from "./community_window/CommunityMainPane";
import CommunityModals from "./community_window/CommunityModals";
import CommunityRightPanel from "./community_window/CommunityRightPanel";
import CommunitySidebar from "./community_window/CommunitySidebar";
import { useCommunityWindow } from "./community_window/useCommunityWindow";

const CommunityWindow: Component = () => {
  const vm = useCommunityWindow();
  // The window's kind (plan SB1), as its label names it: the browser is the
  // community window opened with no community (`/community`, no `id`).
  const kind = new URLSearchParams(window.location.search).get("id")
    ? "community"
    : "community-browser";

  setWindowKind(kind);
  const title = (): string => vm.activeCommunity()?.name ?? "Community";
  return (
    <SlotProvider>
      {/* `data-skin-window` names the pack frame the Host composes for a
        community window (plan SKW1: the pack's chat window). */}
      <div
        class="app-frame community-window"
        data-skin-window={kind === "community" ? "community" : undefined}
      >
        {/* Headless: keeps the community voice/video WebCodecs pipeline alive
          across channel navigation, publishing it to the main-pane CallStage. */}
        <CallPipelineHost communityId={vm.selectedCommunityId()} />
        {/* Architecture §32 a11y — keyboard skip link past the navigation
          rails; MessageList's container carries id="main-content". */}
        <a href="#main-content" class="skip-link">
          Skip to messages
        </a>
        {/* A community window wears the pack's frame (plan SKW1): its title
          bar, window buttons, menus and borders, the community's name in its
          chatTitle node; the rails and messages mount into its chat area and
          the composer into its edit slot. The browser, and a pack serving
          none, keep the app's own titlebar. */}
        <Show
          when={kind === "community" ? skinShell("community") : undefined}
          keyed
          fallback={<Titlebar title={title()} showMaximize />}
        >
          {(shell) => (
            <SkinTreeWindow
              shell={shell}
              views={() => null}
              communities={() => null}
              chatTitle={title}
            />
          )}
        </Show>
        <SlotMount slot="chatArea">
          {/* Plan E3.2 S9 (A/B R5-2): our own removal, from the live event or
          the community's stored `removed` state, so it survives a restart. */}
          <Show when={vm.ownRemoval()}>
            {(removed) => (
              <div class="removal-banner" role="alert">
                <span class="removal-banner-text">
                  {removalText(vm.activeCommunity()?.name ?? "", removed())}
                </span>
              </div>
            )}
          </Show>
          {/* Plan JW: a join the inviter refused after it returned, in the
          Host's words; the window stays, with its normal close control. */}
          <Show when={vm.joinWithdrawn()}>
            {(text) => (
              <div class="removal-banner" role="alert">
                <span class="removal-banner-text">{text()}</span>
              </div>
            )}
          </Show>
          {/* FX3: no local key mint; while a rotated key is missing we wait for a
          member who holds it, and say so. Nothing is blocked. */}
          <Show when={vm.keyPending()}>
            {(k) => (
              <div class="key-pending-notice" role="status">
                Waiting for the community key from a member…
                <Show when={k().sinceMs}>
                  {(since) => (
                    <span class="key-pending-since"> since {formatTimestamp(since())}</span>
                  )}
                </Show>
              </div>
            )}
          </Show>
          <div class="community-layout">
            <CommunitySidebar vm={vm} />
            <CommunityMainPane vm={vm} />
            <CommunityRightPanel vm={vm} />
          </div>
        </SlotMount>

        <CommunityModals vm={vm} />
      </div>
    </SlotProvider>
  );
};

export default CommunityWindow;

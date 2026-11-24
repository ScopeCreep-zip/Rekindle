import { Component, Show } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import { removalText } from "../utils/removal_text";
import { useCommunityWindow } from "./community_window/useCommunityWindow";
import CommunitySidebar from "./community_window/CommunitySidebar";
import CommunityMainPane from "./community_window/CommunityMainPane";
import CommunityRightPanel from "./community_window/CommunityRightPanel";
import CommunityModals from "./community_window/CommunityModals";
import CallPipelineHost from "../components/voice/call_stage/CallPipelineHost";

const CommunityWindow: Component = () => {
  const vm = useCommunityWindow();
  // The window's kind (plan SB1), as its label names it: the browser is the
  // community window opened with no community (`/community`, no `id`).
  const kind = new URLSearchParams(window.location.search).get("id") ? "community" : "community-browser";

  return (
    <div class="app-frame community-window" data-window={kind}>
      {/* Headless: keeps the community voice/video WebCodecs pipeline alive
          across channel navigation, publishing it to the main-pane CallStage. */}
      <CallPipelineHost communityId={vm.selectedCommunityId()} />
      {/* Architecture §32 a11y — keyboard skip link past the navigation
          rails; MessageList's container carries id="main-content". */}
      <a href="#main-content" class="skip-link">Skip to messages</a>
      <Titlebar title={vm.activeCommunity()?.name ?? "Community"} showMaximize />
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
      <div class="community-layout">
        <CommunitySidebar vm={vm} />
        <CommunityMainPane vm={vm} />
        <CommunityRightPanel vm={vm} />
      </div>

      <CommunityModals vm={vm} />
    </div>
  );
};

export default CommunityWindow;

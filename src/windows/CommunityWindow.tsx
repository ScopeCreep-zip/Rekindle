import { Component, Show } from "solid-js";
import Titlebar from "../components/titlebar/Titlebar";
import ToastContainer from "../components/common/Toast";
import { setCommunityState } from "../stores/community.store";
import { useCommunityWindow } from "./community_window/useCommunityWindow";
import CommunitySidebar from "./community_window/CommunitySidebar";
import CommunityMainPane from "./community_window/CommunityMainPane";
import CommunityRightPanel from "./community_window/CommunityRightPanel";
import CommunityModals from "./community_window/CommunityModals";
import CallPipelineHost from "../components/voice/call_stage/CallPipelineHost";

const CommunityWindow: Component = () => {
  const vm = useCommunityWindow();

  return (
    <div class="app-frame">
      {/* Headless: keeps the community voice/video WebCodecs pipeline alive
          across channel navigation, publishing it to the main-pane CallStage. */}
      <CallPipelineHost communityId={vm.selectedCommunityId()} />
      {/* Architecture §32 a11y — keyboard skip link past the navigation
          rails; MessageList's container carries id="main-content". */}
      <a href="#main-content" class="skip-link">Skip to messages</a>
      <Titlebar title={vm.activeCommunity()?.name ?? "Community"} showMaximize />
      {/* Architecture §17.4 / Plan §Failure 11 — raid alert banner stays
          until a moderator dismisses; the detector re-trips if the
          join-flood continues next interval. */}
      <Show when={vm.activeCommunity()?.raidAlertActive}>
        <div class="raid-alert-banner" role="alert">
          <span class="raid-alert-banner-text">
            <strong>Raid detected</strong> — slowmode and join-throttling are
            active. Moderators can review recent joins and pause invites.
          </span>
          <button
            type="button"
            class="form-btn-secondary"
            onClick={() => {
              const id = vm.selectedCommunityId();
              if (id) {
                setCommunityState("communities", id, "raidAlertActive", false);
              }
            }}
          >
            Dismiss
          </button>
        </div>
      </Show>
      <div class="community-layout">
        <CommunitySidebar vm={vm} />
        <CommunityMainPane vm={vm} />
        <CommunityRightPanel vm={vm} />
      </div>

      <CommunityModals vm={vm} />
      <ToastContainer />
    </div>
  );
};

export default CommunityWindow;

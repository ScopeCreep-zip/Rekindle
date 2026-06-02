import { Component, For, Show } from "solid-js";
import ParticipantTile from "./ParticipantTile";
import CallControlBar from "./CallControlBar";
import { useCallStage } from "./useCallStage";
import { activePipeline } from "./pipeline_store";
import ChannelChat from "../../chat/ChannelChat";
import { handleJoinVoice } from "../../../handlers/voice.handlers";
import { ICON_PHONE } from "../../../icons";
import type { CommunityVm } from "../../../windows/community_window/useCommunityWindow";

/// Main-pane conferencing surface for a community voice channel. When the
/// local member isn't connected it shows a Join hero; once connected it
/// becomes a participant gallery (with screen-share / click-to-spotlight)
/// over a centred control bar, plus an optional §10.8 text-in-voice chat
/// column. Reuses the live WebCodecs pipeline published by CallPipelineHost.
const CallStage: Component<{ vm: CommunityVm }> = (props) => {
  const vm = props.vm;
  const stage = useCallStage();
  const connected = (): boolean => vm.isConnectedToActiveStage();

  return (
    <div class="call-stage">
      <Show
        when={connected()}
        fallback={
          <div class="call-stage-hero">
            <span class="nf-icon call-stage-hero-icon" aria-hidden="true">{ICON_PHONE}</span>
            <div class="call-stage-hero-title">{vm.activeChannel()?.name}</div>
            <div class="call-stage-hero-subtitle">
              Voice channel — join to talk, turn on your camera, or share your screen.
            </div>
            <button
              class="call-stage-join-btn"
              onClick={() => void handleJoinVoice(vm.selectedChannelId(), vm.selectedCommunityId())}
            >
              <span class="nf-icon" aria-hidden="true">{ICON_PHONE}</span> Join Voice
            </button>
          </div>
        }
      >
        <div class="call-stage-body" classList={{ "call-stage-body-with-chat": vm.showCallChat() }}>
          <div class="call-stage-stage">
            <Show
              when={stage.spotlight()}
              fallback={
                <div class="call-stage-gallery">
                  <For each={stage.tiles()}>
                    {(tile) => (
                      <ParticipantTile tile={tile} onSpotlight={() => stage.toggleSpotlight(tile.key)} />
                    )}
                  </For>
                </div>
              }
            >
              <div class="call-stage-spotlight-wrap">
                <ParticipantTile
                  tile={stage.spotlight()!}
                  spotlighted
                  onSpotlight={() => stage.toggleSpotlight(stage.spotlight()!.key)}
                />
                <Show when={stage.filmstrip().length > 0}>
                  <div class="call-stage-filmstrip">
                    <For each={stage.filmstrip()}>
                      {(tile) => (
                        <ParticipantTile tile={tile} onSpotlight={() => stage.toggleSpotlight(tile.key)} />
                      )}
                    </For>
                  </div>
                </Show>
              </div>
            </Show>
            <CallControlBar
              chatOpen={vm.showCallChat()}
              onToggleChat={() => vm.setShowCallChat(!vm.showCallChat())}
              onPip={() => void activePipeline()?.togglePictureInPicture()}
            />
          </div>

          <Show when={vm.showCallChat()}>
            <div class="call-stage-chat">
              <ChannelChat vm={vm} />
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default CallStage;

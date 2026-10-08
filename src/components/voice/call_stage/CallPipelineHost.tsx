import { Component, Show, createMemo, onMount, onCleanup } from "solid-js";
import { voiceState } from "../../../stores/voice.store";
import { useVideoCallPanel } from "../video_call/useVideoCallPanel";
import { setActivePipeline } from "./pipeline_store";

/// Instantiates the WebCodecs pipeline for one community voice channel and
/// publishes it to the module-level `activePipeline` signal. Kept in its own
/// keyed instance so switching voice channels tears down and re-creates the
/// pipeline cleanly (registering/unregistering the per-stream IPC channels).
const PipelineInstance: Component<{ communityId: string; channelId: string }> = (props) => {
  const vm = useVideoCallPanel({
    mode: "community",
    communityId: props.communityId,
    channelId: props.channelId,
    visible: true,
  });
  onMount(() => setActivePipeline(vm));
  onCleanup(() => setActivePipeline(null));
  return null;
};

/// Headless host mounted once at the community-window root. While the local
/// member is connected to a community voice channel it keeps the pipeline
/// alive regardless of which channel the main pane is showing — so the
/// camera/screen-share keeps broadcasting when you browse other channels.
const CallPipelineHost: Component<{ communityId: string }> = (props) => {
  // Re-key on the connected channel id so changing voice channels remounts
  // the inner instance. Returns null when no community call is active.
  const activeChannelId = createMemo(() => {
    const connected =
      voiceState.isConnected
      && voiceState.activeCallType === "community"
      && voiceState.channelId != null
      && props.communityId !== "";
    return connected ? voiceState.channelId : null;
  });

  return (
    <Show when={activeChannelId()} keyed>
      {(channelId) => (
        <PipelineInstance communityId={props.communityId} channelId={channelId} />
      )}
    </Show>
  );
};

export default CallPipelineHost;

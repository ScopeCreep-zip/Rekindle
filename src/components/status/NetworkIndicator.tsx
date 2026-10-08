import { Component, createSignal, onMount, onCleanup } from "solid-js";
import { handleGetNetworkStatus } from "../../actions/settings.actions";
import { subscribeNetworkStatus } from "../../ipc/channels";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { RouteAvailability } from "../../ipc/channels/notification_events";

const NetworkIndicator: Component = () => {
  const [attachmentState, setAttachmentState] = createSignal("detached");
  const [isAttached, setIsAttached] = createSignal(false);
  const [publicInternetReady, setPublicInternetReady] = createSignal(false);
  const [hasRoute, setHasRoute] = createSignal(false);
  const [mediaRoute, setMediaRoute] = createSignal<RouteAvailability>("idle");

  let pollTimer: ReturnType<typeof setInterval> | undefined;
  let unlistenNetwork: UnlistenFn | undefined;

  async function pollNetworkStatus(): Promise<void> {
    const status = await handleGetNetworkStatus();
    if (status) {
      setAttachmentState(status.attachmentState);
      setIsAttached(status.isAttached);
      setPublicInternetReady(status.publicInternetReady);
      setHasRoute(status.hasRoute);
      setMediaRoute(status.mediaRoute);
    }
  }

  onMount(() => {
    // Initial fetch + polling safety net (every 30s since push events are authoritative)
    pollNetworkStatus();
    pollTimer = setInterval(pollNetworkStatus, 30000);

    // Subscribe to push events for instant updates
    subscribeNetworkStatus((event) => {
      setAttachmentState(event.attachmentState);
      setIsAttached(event.isAttached);
      setPublicInternetReady(event.publicInternetReady);
      setHasRoute(event.hasRoute);
      setMediaRoute(event.mediaRoute);
    }).then((fn) => {
      unlistenNetwork = fn;
    });
  });

  onCleanup(() => {
    if (pollTimer) {
      clearInterval(pollTimer);
    }
    if (unlistenNetwork) {
      unlistenNetwork();
    }
  });

  function dotClass(): string {
    if (publicInternetReady() && hasRoute()) return "network-dot network-dot-connected";
    if (isConnecting()) return "network-dot network-dot-connecting";
    if (publicInternetReady() && !hasRoute()) return "network-dot network-dot-partial";
    return "network-dot network-dot-disconnected";
  }

  function isConnecting(): boolean {
    const state = attachmentState();
    return state === "attaching" || state === "attached_weak" || (isAttached() && !publicInternetReady());
  }

  function label(): string {
    if (publicInternetReady() && hasRoute()) return "Connected";
    if (isConnecting()) return "Connecting...";
    if (publicInternetReady() && !hasRoute()) return "No Route";
    return "Disconnected";
  }

  /**
   * Voice and video need the media route, which the general route never
   * stands in for (plan C7.9c), so while connected without it the
   * indicator says so. `null` once it is available.
   */
  function voiceNote(): string | null {
    if (!(publicInternetReady() && hasRoute())) return null;
    switch (mediaRoute()) {
      case "available":
        return null;
      case "failed":
        return "Voice unavailable";
      default:
        return "Voice connecting...";
    }
  }

  function title(): string {
    const note = voiceNote();
    return note ? `Veilid: ${label()} — ${note}` : `Veilid: ${label()}`;
  }

  return (
    <div class="network-indicator" title={title()}>
      <div class={dotClass()} />
      <span>{label()}</span>
      {voiceNote() && <span class="network-voice-note">{voiceNote()}</span>}
    </div>
  );
};

export default NetworkIndicator;

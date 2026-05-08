import { Component, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import IncomingCallModal from "./IncomingCallModal";
import CallWaitingBanner from "./CallWaitingBanner";
import OutgoingCallPanel from "./OutgoingCallPanel";
import GroupCallPanel from "./GroupCallPanel";
import { refreshMissedCalls, subscribeCallEvents } from "../../handlers/calls.handlers";
import { subscribeNotificationHandler } from "../../handlers/notification-events.handlers";

// Wave 12 W12.1 — Global call/notification subscription host. Mounted in
// every webview by main.tsx so ringing/banner/modal works regardless of
// which window currently has focus. Previously these listeners lived in
// BuddyListWindow only, so calls only surfaced when the buddy list was
// the active window — broken when the user was in DmWindow / ChatWindow /
// CommunityWindow / ProfileWindow / SettingsWindow.
//
// The IncomingCallModal renders in this slot. The OutgoingCallPanel and
// CallWaitingBanner mount here too once W12.3 / W12.4 land.
const CallController: Component = () => {
  const unlisteners: Promise<UnlistenFn>[] = [];

  onMount(() => {
    unlisteners.push(subscribeCallEvents());
    unlisteners.push(subscribeNotificationHandler());
    void refreshMissedCalls();
  });

  onCleanup(() => {
    for (const p of unlisteners) {
      p.then((fn) => fn()).catch(() => {});
    }
  });

  return (
    <>
      <IncomingCallModal />
      <CallWaitingBanner />
      <OutgoingCallPanel />
      <GroupCallPanel />
    </>
  );
};

export default CallController;

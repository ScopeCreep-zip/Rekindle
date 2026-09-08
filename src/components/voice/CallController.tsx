import { Component, Show } from "solid-js";
import IncomingCallModal from "./IncomingCallModal";
import CallWaitingBanner from "./CallWaitingBanner";
import OutgoingCallPanel from "./OutgoingCallPanel";
import GroupCallPanel from "./GroupCallPanel";
import ActiveCallPanel from "./ActiveCallPanel";
import { callsState } from "../../stores/calls.store";

// Wave 12 W12.1 — Global call/notification subscription host. Mounted in
// every webview by main.tsx so ringing/banner/modal works regardless of
// which window currently has focus. Previously these listeners lived in
// BuddyListWindow only, so calls only surfaced when the buddy list was
// the active window — broken when the user was in DmWindow / ChatWindow /
// CommunityWindow / ProfileWindow / SettingsWindow.
//
// The IncomingCallModal renders in this slot. The OutgoingCallPanel and
// CallWaitingBanner mount here too once W12.3 / W12.4 land.
//
// The three subscriptions this used to register in its own `onMount`
// now run in `main.tsx`, which is where app-start event wiring belongs
// — a component registering them made this the last thing in
// `src/components/` reaching into `src/handlers/`. Their lifetime is
// unchanged: main.tsx mounts this host in every webview.
const CallController: Component = () => {
  return (
    <>
      <IncomingCallModal />
      <CallWaitingBanner />
      <OutgoingCallPanel />
      <GroupCallPanel />
      {/* W13-fix.2 — globally-mounted active-call panel so the receiver
       *  sees call controls (timer, mute, hangup, etc.) immediately
       *  after Accept, regardless of which window they're in. Was
       *  only mounted inside DmWindow before — meaning if you accepted
       *  from anywhere else (BuddyList, ChatWindow, ProfileWindow),
       *  the modal disappeared and you saw NOTHING. */}
      <Show when={callsState.activeCall}>
        {(call) => (
          <div class="global-active-call-overlay">
            <ActiveCallPanel call={call()} mode="inline" />
          </div>
        )}
      </Show>
    </>
  );
};

export default CallController;

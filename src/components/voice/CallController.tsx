import { Component } from "solid-js";
import IncomingCallModal from "./IncomingCallModal";
import CallWaitingBanner from "./CallWaitingBanner";
import OutgoingCallPanel from "./OutgoingCallPanel";
import GroupCallPanel from "./GroupCallPanel";

// The call shell, mounted once in the buddy list: the incoming-call
// modal, the call-waiting banner and the outgoing and group panels. A
// connected 1:1 call opens its own `call-*` window (the backend's
// `present_active_call`), which is the only place its controls and media
// pipeline run — one window, one camera pipeline.
const CallController: Component = () => {
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

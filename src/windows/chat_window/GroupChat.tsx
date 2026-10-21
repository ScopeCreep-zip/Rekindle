import { Component } from "solid-js";
import Titlebar from "../../components/titlebar/Titlebar";
import MessageList from "../../components/chat/MessageList";
import { authState } from "../../stores/auth.store";

/// The chat window's group mode. Group DMs are `rekindle-dm`'s signed
/// rosters and sender-owned keys (plan E2.6); the SMPL group path and its
/// commands are gone, and the group requests, events and this window's
/// wiring are slice G3's. Until then it renders an empty group.
const GroupChat: Component<{ recordKey: string }> = () => (
  <>
    <Titlebar title="Chat — Group DM" showMaximize />
    <div id="main-content" tabindex="-1" class="window-main">
      <MessageList messages={[]} ownName={authState.displayName ?? "You"} peerName="Group DM" />
      <div class="typing-indicator" role="status">
        <span class="typing-label">Group chats are not available yet.</span>
      </div>
    </div>
  </>
);

export default GroupChat;

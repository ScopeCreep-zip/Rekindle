import { Component, Match, Switch } from "solid-js";
import SkinFrame from "../components/common/SkinFrame";
import Titlebar from "../components/titlebar/Titlebar";
import DirectChat from "./chat_window/DirectChat";
import GroupChat from "./chat_window/GroupChat";

/// The one chat window (plan E2.3): `?peer=<identity key>` is a 1:1
/// conversation, `?group=<group id>` a group DM (32 hex, plan E2.6
/// ). The backend builds both URLs (`windows::open_chat_window`).
const ChatWindow: Component = () => {
  const params = new URLSearchParams(window.location.search);
  const peer = params.get("peer") ?? "";
  const group = params.get("group") ?? "";

  return (
    // `chat-window` scopes the chat window's skin surfaces (plan E7.7.7).
    // `data-skin-window` names the skin's chat frame layout (plan SK0 S2).
    <div class="app-frame chat-window" data-skin-window="chat">
      <SkinFrame />
      {/* Architecture §32 a11y — keyboard skip link past the header. */}
      <a href="#main-content" class="skip-link">Skip to messages</a>
      <Switch
        fallback={
          <>
            <Titlebar title="Chat" showMaximize />
            <div class="empty-placeholder">
              <div class="empty-placeholder-title">No conversation selected</div>
              <div class="empty-placeholder-subtitle">
                Open a chat from the buddy list.
              </div>
            </div>
          </>
        }
      >
        <Match when={group}>
          <GroupChat groupId={group} />
        </Match>
        <Match when={peer}>
          <DirectChat peerId={peer} />
        </Match>
      </Switch>
    </div>
  );
};

export default ChatWindow;

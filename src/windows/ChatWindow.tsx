import { Component, Match, Switch, onCleanup, onMount } from "solid-js";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeSkinSignals } from "../ipc/channels";
import { setSkinSignals } from "../stores/skin-signals.store";
import SkinFrame from "../components/common/SkinFrame";
import Titlebar from "../components/titlebar/Titlebar";
import DirectChat from "./chat_window/DirectChat";
import GroupChat from "./chat_window/GroupChat";
import { SlotProvider } from "../components/skin-tree/slots";
import { useChatSearchChord } from "../hooks/useChatSearchChord";
import { setWindowKind } from "../utils/window_kind";
import { registerShellRunner } from "../actions/skin.actions";
import { callOps, friendOps, menuOps, muteOps, popupOps } from "../actions/shell_runners";

/// The one chat window (plan E2.3): `?peer=<identity key>` is a 1:1
/// conversation, `?group=<group id>` a group DM (32 hex, plan E2.6
/// ). The backend builds both URLs (`windows::open_chat_window`).
const ChatWindow: Component = () => {
  const params = new URLSearchParams(window.location.search);
  const peer = params.get("peer") ?? "";
  const group = params.get("group") ?? "";
  useChatSearchChord(group ? null : peer || null);
  // The Host's signals for this window (skin-signals:<label>): a chat
  // tree's state triggers read them, its talkingState among them.
  let unlisten: UnlistenFn | undefined;
  // The groups the chat trees serve (shell_runners): the File, Tool and
  // Help menus, a pack popup, the friend's profile, a call and mute. No
  // friend chat, room or download: no chat tree lays one.
  const unregister = [menuOps, popupOps, friendOps, callOps, muteOps].map((run) => registerShellRunner(run));
  onMount(async () => {
    unlisten = await subscribeSkinSignals(setSkinSignals);
  });
  onCleanup(() => {
    unlisten?.();
    for (const off of unregister) off();
  });

  // The window's kind (plan SB1), as its label names it: a group's or a 1:1's.
  setWindowKind(group ? "group-chat" : "chat");
  return (
    // `chat-window` scopes the chat window's skin surfaces (plan E7.7.7).
    // `data-skin-window` names the skin's chat frame layout (plan SK0 S2).
    <SlotProvider>
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
    </SlotProvider>
  );
};

export default ChatWindow;

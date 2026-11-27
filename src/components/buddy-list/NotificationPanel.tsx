import { type Component, createEffect, For, on, onCleanup, Show } from "solid-js";
import { handleStartDmCall } from "../../actions/calls.actions";
import { ICON_CHECK, ICON_PHONE, ICON_SEND } from "../../icons";
import { commands } from "../../ipc/commands";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import {
  markAllNotificationsRead,
  markNotificationRead,
  notificationState,
  unreadNotifications,
} from "../../stores/notification.store";
import { formatRelativeTime } from "../../utils/formatting";

/**
 * The notification inbox (contract alerts, 2026-11-27): shown while
 * `buddyListUI.showNotifications` is set, by the Host's `openPanel`
 * (a pack's notifications control, the Rekindle menu's Notifications, the
 * stock bell, each through `openNotifications`, so the Host acknowledges
 * its unread bit). Mounted once in the buddy list beside its skin tree,
 * so it shows in the stock layout and under every pack alike, a card
 * centred in the window. Escape or a press outside closes it and gives
 * focus back where it was; closing is the shell's own (the bit stays as
 * the Host left it).
 */
const NotificationPanel: Component = () => {
  let panel: HTMLDivElement | undefined;
  let returnTo: HTMLElement | null = null;
  const close = (): void => {
    setBuddyListUI("showNotifications", false);
    returnTo?.focus();
    returnTo = null;
  };
  const onPointer = (e: MouseEvent): void => {
    const target = e.target as Element | null;
    // The stock bell toggles the panel itself.
    if (panel && target && !panel.contains(target) && !target.closest(".notification-bell"))
      close();
  };
  const onKey = (e: KeyboardEvent): void => {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };
  createEffect(
    on(
      () => buddyListUI.showNotifications,
      (open) => {
        if (!open) return;
        returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        queueMicrotask(() => panel?.focus());
        document.addEventListener("mousedown", onPointer);
        onCleanup(() => document.removeEventListener("mousedown", onPointer));
      }
    )
  );

  return (
    <Show when={buddyListUI.showNotifications}>
      <div
        ref={panel}
        class="notification-panel"
        role="dialog"
        aria-label="Notifications"
        tabindex="-1"
        onKeyDown={onKey}
      >
        <div class="notification-panel-header">
          <span class="notification-panel-title">Notifications</span>
          <Show when={unreadNotifications() > 0}>
            <button type="button" class="notification-mark-all" onClick={markAllNotificationsRead}>
              Mark all read
            </button>
          </Show>
        </div>
        <div class="notification-panel-list">
          <Show
            when={notificationState.notifications.length > 0}
            fallback={<div class="notification-empty">No notifications</div>}
          >
            <For each={notificationState.notifications}>
              {(notification) => (
                <div
                  class={
                    notification.read
                      ? "notification-item notification-item-read"
                      : "notification-item"
                  }
                >
                  <div class="notification-item-content">
                    <span class="notification-title">{notification.title}</span>
                    <span class="notification-body">{notification.body}</span>
                    <span class="notification-time">
                      {formatRelativeTime(notification.timestamp)}
                    </span>
                    {/* Wave 12 W12.8 — missed-call action row. */}
                    <Show
                      when={notification.type === "missed_call" && notification.peerKey != null}
                    >
                      <div class="notification-actions">
                        <button
                          type="button"
                          class="form-btn-secondary notification-action-btn"
                          title="Call back"
                          onClick={() => {
                            const peerKey = notification.peerKey!;
                            const name = notification.body.split(" (")[0];
                            const kind = notification.callKind ?? "audio";
                            void handleStartDmCall(peerKey, name, kind === "video");
                            markNotificationRead(notification.id);
                          }}
                        >
                          <span class="nf-icon" aria-hidden="true">
                            {ICON_PHONE}
                          </span>
                          Call back
                        </button>
                        <button
                          type="button"
                          class="form-btn-secondary notification-action-btn"
                          title="Send a message"
                          onClick={() => {
                            const peerKey = notification.peerKey!;
                            void commands.openChatWindow(peerKey);
                            markNotificationRead(notification.id);
                          }}
                        >
                          <span class="nf-icon" aria-hidden="true">
                            {ICON_SEND}
                          </span>
                          Message
                        </button>
                      </div>
                    </Show>
                  </div>
                  <Show when={!notification.read}>
                    <button
                      type="button"
                      class="notification-mark-read"
                      onClick={() => markNotificationRead(notification.id)}
                      title="Mark as read"
                      aria-label="Mark notification as read"
                    >
                      <span class="nf-icon nf-icon-sm" aria-hidden="true">
                        {ICON_CHECK}
                      </span>
                    </button>
                  </Show>
                </div>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
};

export default NotificationPanel;

import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import {
  notificationState,
  markNotificationRead,
  markAllNotificationsRead,
} from "../../stores/notification.store";
import { ICON_BELL, ICON_CHECK, ICON_PHONE, ICON_SEND } from "../../icons";
import { formatRelativeTime } from "../../utils/formatting";
import { handleStartDmCall } from "../../handlers/calls.handlers";
import { commands } from "../../ipc/commands";

const NotificationCenter: Component = () => {
  const [open, setOpen] = createSignal(false);
  let panelRef: HTMLDivElement | undefined;

  function toggle(): void {
    setOpen((prev) => !prev);
  }

  function handleClickOutside(e: MouseEvent): void {
    if (panelRef && !panelRef.contains(e.target as Node)) {
      setOpen(false);
    }
  }

  onMount(() => {
    document.addEventListener("mousedown", handleClickOutside);
  });

  onCleanup(() => {
    document.removeEventListener("mousedown", handleClickOutside);
  });

  return (
    <div class="notification-bell-wrapper" ref={panelRef}>
      <button
        class="notification-bell"
        onClick={toggle}
        title="Notifications"
        aria-label={
          notificationState.unreadCount > 0
            ? `Notifications, ${notificationState.unreadCount} unread`
            : "Notifications"
        }
        aria-expanded={open()}
      >
        <span class="nf-icon nf-icon-md" aria-hidden="true">{ICON_BELL}</span>
        <Show when={notificationState.unreadCount > 0}>
          <span class="notification-badge">
            {notificationState.unreadCount > 99
              ? "99+"
              : notificationState.unreadCount}
          </span>
        </Show>
      </button>

      <Show when={open()}>
        <div class="notification-panel">
          <div class="notification-panel-header">
            <span class="notification-panel-title">Notifications</span>
            <Show when={notificationState.unreadCount > 0}>
              <button
                class="notification-mark-all"
                onClick={markAllNotificationsRead}
              >
                Mark all read
              </button>
            </Show>
          </div>

          <div class="notification-panel-list">
            <Show
              when={notificationState.notifications.length > 0}
              fallback={
                <div class="notification-empty">No notifications</div>
              }
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
                      <span class="notification-title">
                        {notification.title}
                      </span>
                      <span class="notification-body">
                        {notification.body}
                      </span>
                      <span class="notification-time">
                        {formatRelativeTime(notification.timestamp)}
                      </span>
                      {/* Wave 12 W12.8 — missed-call action row. */}
                      <Show
                        when={
                          notification.type === "missed_call" &&
                          notification.peerKey != null
                        }
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
                            <span class="nf-icon" aria-hidden="true">{ICON_PHONE}</span>
                            Call back
                          </button>
                          <button
                            type="button"
                            class="form-btn-secondary notification-action-btn"
                            title="Send a message"
                            onClick={() => {
                              const peerKey = notification.peerKey!;
                              const name = notification.body.split(" (")[0];
                              void commands.openChatWindow(peerKey, name);
                              markNotificationRead(notification.id);
                            }}
                          >
                            <span class="nf-icon" aria-hidden="true">{ICON_SEND}</span>
                            Message
                          </button>
                        </div>
                      </Show>
                    </div>
                    <Show when={!notification.read}>
                      <button
                        class="notification-mark-read"
                        onClick={() => markNotificationRead(notification.id)}
                        title="Mark as read"
                        aria-label="Mark notification as read"
                      >
                        <span class="nf-icon nf-icon-sm" aria-hidden="true">{ICON_CHECK}</span>
                      </button>
                    </Show>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default NotificationCenter;

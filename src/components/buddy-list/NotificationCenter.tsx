import { Component, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import {
  notificationState,
  markNotificationRead,
  markAllNotificationsRead,
} from "../../stores/notification.store";
import { ICON_BELL, ICON_CHECK } from "../../icons";
import { formatRelativeTime } from "../../utils/formatting";

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
      <button class="notification-bell" onClick={toggle} title="Notifications">
        <span class="nf-icon nf-icon-md">{ICON_BELL}</span>
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
                    </div>
                    <Show when={!notification.read}>
                      <button
                        class="notification-mark-read"
                        onClick={() => markNotificationRead(notification.id)}
                        title="Mark as read"
                      >
                        <span class="nf-icon nf-icon-sm">{ICON_CHECK}</span>
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

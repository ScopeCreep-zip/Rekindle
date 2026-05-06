import { createStore } from "solid-js/store";

export interface Notification {
  id: string;
  type: "message" | "friend_request" | "community_invite" | "system";
  title: string;
  body: string;
  timestamp: number;
  read: boolean;
  /** Set when type === "message" — the originating community. */
  communityId?: string;
  /** Set when type === "message" — the originating channel. */
  channelId?: string;
  /**
   * Architecture §32 Phase 7 Week 25 — resolved per-channel /
   * per-community sound override. `null` means use the bundled
   * default. Only populated for `type === "message"` notifications.
   */
  soundRef?: string | null;
}

export interface NotificationState {
  notifications: Notification[];
  unreadCount: number;
}

const [notificationState, setNotificationState] =
  createStore<NotificationState>({
    notifications: [],
    unreadCount: 0,
  });

export function markNotificationRead(id: string): void {
  const idx = notificationState.notifications.findIndex((n) => n.id === id);
  if (idx >= 0 && !notificationState.notifications[idx].read) {
    setNotificationState("notifications", idx, "read", true);
    setNotificationState("unreadCount", (c) => Math.max(0, c - 1));
  }
}

export function markAllNotificationsRead(): void {
  setNotificationState(
    "notifications",
    (n) => !n.read,
    "read",
    true,
  );
  setNotificationState("unreadCount", 0);
}

export { notificationState, setNotificationState };

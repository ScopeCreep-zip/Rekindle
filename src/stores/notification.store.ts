import { createStore } from "solid-js/store";

export interface Notification {
  id: string;
  type: "message" | "friend_request" | "community_invite" | "system";
  title: string;
  body: string;
  timestamp: number;
  read: boolean;
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

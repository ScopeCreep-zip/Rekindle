import { createStore } from "solid-js/store";

export interface Notification {
  id: string;
  /**
   * Wave 12 W12.8 — `missed_call` rows surface in the notification panel
   * with Call-back / Send Message action buttons. They're created when
   * a `chat-event::callMissed` (callee never answered) or
   * `chat-event::callTimedOut` (caller's offer expired) fires.
   */
  type:
    | "message"
    | "friend_request"
    | "community_invite"
    | "system"
    | "missed_call";
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
  /**
   * Wave 12 W12.3/W12.8 — for call-related notifications, the
   * originating `call_id` so the missed-call panel can offer a
   * Call-back action without re-querying.
   */
  callId?: string;
  /**
   * Wave 12 W12.8 — peer pubkey for `missed_call` rows so the
   * Call-back / Send Message actions know whom to invoke.
   */
  peerKey?: string;
  /** Wave 12 W12.8 — preserved kind so Call-back uses the same media. */
  callKind?: "audio" | "video";
}

export interface NotificationState {
  notifications: Notification[];
}

const [notificationState, setNotificationState] =
  createStore<NotificationState>({
    notifications: [],
  });

/// The inbox rows not yet read. Message, channel and DM counts are the
/// Host's (`UnreadChanged`, plan E3.1 D25); this store keeps no counter.
export function unreadNotifications(): number {
  return notificationState.notifications.filter((n) => !n.read).length;
}

export function markNotificationRead(id: string): void {
  const idx = notificationState.notifications.findIndex((n) => n.id === id);
  if (idx >= 0 && !notificationState.notifications[idx].read) {
    setNotificationState("notifications", idx, "read", true);
  }
}

export function markAllNotificationsRead(): void {
  setNotificationState(
    "notifications",
    (n) => !n.read,
    "read",
    true,
  );
}

export { notificationState, setNotificationState };

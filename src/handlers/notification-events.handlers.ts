import type { UnlistenFn } from "@tauri-apps/api/event";
import { subscribeNotificationEvents } from "../ipc/channels";
import { setNotificationState } from "../stores/notification.store";

export function subscribeNotificationHandler(): Promise<UnlistenFn> {
  return subscribeNotificationEvents((event) => {
    switch (event.type) {
      case "systemAlert": {
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "system",
            title: event.data.title,
            body: event.data.body,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
      case "updateAvailable": {
        setNotificationState("notifications", (prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            type: "system",
            title: "Update Available",
            body: `Version ${event.data.version} is available`,
            timestamp: Date.now(),
            read: false,
          },
        ]);
        setNotificationState("unreadCount", (c) => c + 1);
        break;
      }
    }
  });
}

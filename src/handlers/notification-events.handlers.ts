import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { subscribeNotificationEvents } from "../ipc/channels";
import { setNotificationState } from "../stores/notification.store";
import { authState } from "../stores/auth.store";

export async function showSystemNotification(title: string, body: string): Promise<void> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (granted) {
      await sendNotification({ title, body });
    }
  } catch (error) {
    console.warn("Failed to show system notification:", error);
  }
}

export function subscribeNotificationHandler(): Promise<UnlistenFn> {
  return subscribeNotificationEvents((event) => {
    switch (event.type) {
      case "systemAlert": {
        void showSystemNotification(event.data.title, event.data.body);
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
        if (authState.status === "busy") break;
        void showSystemNotification(
          "Update Available",
          `Version ${event.data.version} is available`,
        );
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

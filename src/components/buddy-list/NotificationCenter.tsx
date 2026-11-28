import { type Component, Show } from "solid-js";
import { dispatchSkinAction } from "../../actions/skin.actions";
import { ICON_BELL } from "../../icons";
import { buddyListUI, setBuddyListUI } from "../../stores/buddylist-ui.store";
import { unreadNotifications } from "../../stores/notification.store";

/**
 * The stock layout's notification bell. Opening goes through the Host's
 * `openNotifications`, as a pack's notifications control does, so the Host
 * acknowledges its unread bit (contract alerts, 2026-11-27); closing is the
 * shell's own. The inbox itself is `NotificationPanel`, mounted once in the
 * buddy list.
 */
const NotificationCenter: Component = () => {
  const toggle = (): void => {
    if (buddyListUI.showNotifications) setBuddyListUI("showNotifications", false);
    else void dispatchSkinAction("NotificationBell", { kind: "openNotifications" });
  };
  return (
    <div class="notification-bell-wrapper">
      <button
        type="button"
        class="notification-bell"
        onClick={toggle}
        title="Notifications"
        aria-label={
          unreadNotifications() > 0
            ? `Notifications, ${unreadNotifications()} unread`
            : "Notifications"
        }
        aria-expanded={buddyListUI.showNotifications}
      >
        <span class="nf-icon nf-icon-md" aria-hidden="true">
          {ICON_BELL}
        </span>
        <Show when={unreadNotifications() > 0}>
          <span class="notification-badge">
            {unreadNotifications() > 99 ? "99+" : unreadNotifications()}
          </span>
        </Show>
      </button>
    </div>
  );
};

export default NotificationCenter;

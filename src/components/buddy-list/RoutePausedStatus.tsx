import { Component, Show } from "solid-js";
import { networkState, routePausedText, trackNetworkStatus } from "../../stores/network.store";

/**
 * Our inbound route is paused (owner decision 65, plan RV1): the Host's
 * new-route budget is spent or unreadable, so the route stays down instead
 * of drawing again. A persistent status in the buddy list's status area,
 * with no dismiss, shown exactly while the Host says paused.
 */
const RoutePausedStatus: Component = () => {
  trackNetworkStatus();
  return (
    <Show when={networkState.routePaused}>
      <div class="route-paused-status" role="alert">
        {routePausedText(networkState.routePauseCause)}
      </div>
    </Show>
  );
};

export default RoutePausedStatus;

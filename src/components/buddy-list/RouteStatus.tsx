import { type Component, Show } from "solid-js";
import { routeStatus, trackNetworkStatus } from "../../stores/network.store";

/**
 * Our own route's status in the buddy list's status area, exactly while the
 * Host says so: paused (owner decision 65, plan RV1: the new-route budget is
 * spent or unreadable, so the route stays down), or down while the Host
 * retries (plan F9-A: "Unreachable — retrying since 14:52", the time it went
 * down as a clock time, Veilid's reason as its tooltip and description).
 * Persistent, with no dismiss; it clears when the Host says the route is up.
 */
const RouteStatus: Component = () => {
  trackNetworkStatus();
  return (
    <Show when={routeStatus()}>
      {(status) => (
        <div class="route-paused-status" role="alert" title={status().detail ?? undefined}>
          {status().text}
          <Show when={status().detail}>{(why) => <span class="sr-only">: {why()}</span>}</Show>
        </div>
      )}
    </Show>
  );
};

export default RouteStatus;

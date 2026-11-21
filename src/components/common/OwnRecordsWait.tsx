import { Component, Match, Switch } from "solid-js";
import { networkState, trackNetworkStatus } from "../../stores/network.store";

interface OwnRecordsWaitProps {
  /** A call that names our records is under way (or, in a composer, may
   *  be sent): show the wait while the stage is pending. */
  active: boolean;
}

/**
 * Where our identity's publishing is (plan E2.5 FC3), from the Host's
 * own-records stage (`NetworkEvent::OwnRecordsChanged`): an invite, a
 * friend request or a first message waits, with no deadline, until our
 * profile and mailbox are held, so the window says so rather than freezing.
 * A stage that failed shows its reason. Until the Host has said where the
 * stage is (`unknown`, plan FC4) nothing shows: a stage already held must
 * not flash a wait in a window that just opened.
 */
const OwnRecordsWait: Component<OwnRecordsWaitProps> = (props) => {
  trackNetworkStatus();
  return (
    <Switch>
      <Match when={props.active && networkState.ownRecords === "pending"}>
        <div class="add-friend-code-status" role="status">
          Publishing your identity…
        </div>
      </Match>
      <Match when={props.active && networkState.ownRecords === "failed"}>
        <div class="form-error" role="alert">
          Your identity could not be published: {networkState.ownRecordsError ?? "unknown error"}
        </div>
      </Match>
    </Switch>
  );
};

export default OwnRecordsWait;

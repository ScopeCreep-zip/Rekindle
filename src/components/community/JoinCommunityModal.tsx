import { Component, createEffect } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import JoinProgressStepper from "./JoinProgressStepper";
import { handleJoinCommunity } from "../../actions/community.actions";
import { beginJoinProgress, endJoinProgress, resetJoinProgress } from "../../stores/join.store";

interface JoinCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

const JoinCommunityModal: Component<JoinCommunityModalProps> = (props) => {
  // Clear any prior dial-in steps each time the modal opens so a
  // previous failed attempt isn't shown before the user retries.
  createEffect(() => {
    if (props.isOpen) resetJoinProgress();
  });

  return (
    <SimpleInputModal
      isOpen={props.isOpen}
      title="Join Community"
      onClose={props.onClose}
      onSubmit={(input) => {
        // The backend parses the invite link, gates each join phase under
        // its own timeout and streams `joinProgress` events; there is
        // intentionally no single frontend timeout. We only reset/settle
        // the dial-in stepper.
        beginJoinProgress();
        return handleJoinCommunity(input).finally(() => endJoinProgress());
      }}
      placeholder="rekindle://invite/..."
      submitLabel="Join"
      extra={<JoinProgressStepper />}
    />
  );
};

export default JoinCommunityModal;

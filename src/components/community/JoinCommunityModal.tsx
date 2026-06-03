import { Component, createEffect } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import JoinProgressStepper from "./JoinProgressStepper";
import { handleJoinCommunity } from "../../handlers/community.handlers";
import { beginJoinProgress, endJoinProgress, resetJoinProgress } from "../../stores/join.store";

interface JoinCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

/** Parse a deep link URL: rekindle://invite/{communityId}/{secretsRecordKey}/{inviteCode} (or community://). */
function parseDeepLink(
  input: string,
): { communityId: string; secretsRecordKey: string; inviteCode: string } | null {
  const match = input.match(/^rekindle:\/\/(?:invite|community)\/([^/]+)\/([^/]+)\/([^/]+)\/?$/);
  if (match) return { communityId: match[1], secretsRecordKey: match[2], inviteCode: match[3] };
  return null;
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
      onSubmit={(input, name) => {
        // The backend gates each join phase under its own timeout and
        // streams `joinProgress` events; there is intentionally no single
        // frontend timeout. We only reset/settle the dial-in stepper.
        beginJoinProgress();
        const deepLink = parseDeepLink(input.trim());
        const promise = deepLink
          ? handleJoinCommunity(
              deepLink.communityId,
              name || "Joined community",
              deepLink.inviteCode,
              deepLink.secretsRecordKey,
            )
          : handleJoinCommunity(input, name || input.slice(0, 12) + "...");
        return promise.finally(() => endJoinProgress());
      }}
      placeholder="Invite link or community ID..."
      submitLabel="Join"
      secondaryPlaceholder="Name (optional)"
      extra={<JoinProgressStepper />}
    />
  );
};

export default JoinCommunityModal;

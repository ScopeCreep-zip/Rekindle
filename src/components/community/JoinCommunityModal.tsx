import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleJoinCommunity } from "../../handlers/community.handlers";
import { withTimeout, JOIN_TIMEOUT_MS } from "../../utils/request-timeout";

interface JoinCommunityModalProps {
  isOpen: boolean;
  onClose: () => void;
}

/** Parse a deep link URL: rekindle://invite/{communityId}/{inviteCode} or rekindle://community/{communityId}/{inviteCode} */
function parseDeepLink(input: string): { communityId: string; inviteCode: string } | null {
  const match = input.match(/^rekindle:\/\/(?:invite|community)\/([^/]+)\/([^/]+)\/?$/);
  if (match) return { communityId: match[1], inviteCode: match[2] };
  return null;
}

const JoinCommunityModal: Component<JoinCommunityModalProps> = (props) => (
  <SimpleInputModal
    isOpen={props.isOpen}
    title="Join Community"
    onClose={props.onClose}
    onSubmit={(input, name) => {
      const deepLink = parseDeepLink(input.trim());
      const promise = deepLink
        ? handleJoinCommunity(deepLink.communityId, name || "Joined community", deepLink.inviteCode)
        : handleJoinCommunity(input, name || input.slice(0, 12) + "...");
      return withTimeout(promise, JOIN_TIMEOUT_MS, "Join community");
    }}
    placeholder="Invite link or community ID..."
    submitLabel="Join"
    secondaryPlaceholder="Name (optional)"
  />
);

export default JoinCommunityModal;

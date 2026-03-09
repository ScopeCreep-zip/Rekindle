import { Component } from "solid-js";
import SimpleInputModal from "../common/SimpleInputModal";
import { handleJoinCommunity } from "../../handlers/community.handlers";

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
      if (deepLink) {
        return handleJoinCommunity(deepLink.communityId, name || "Joined community", deepLink.inviteCode);
      }
      // Raw community ID (DHT key)
      return handleJoinCommunity(input, name || input.slice(0, 12) + "...");
    }}
    placeholder="Invite link or community ID..."
    submitLabel="Join"
    secondaryPlaceholder="Name (optional)"
  />
);

export default JoinCommunityModal;

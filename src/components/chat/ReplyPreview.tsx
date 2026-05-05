import { Component, Show } from "solid-js";
import { ICON_CLOSE, ICON_REPLY } from "../../icons";

interface ReplyPreviewProps {
  replyTo: { senderName: string; body: string } | null;
  onDismiss: () => void;
}

const ReplyPreview: Component<ReplyPreviewProps> = (props) => {
  return (
    <Show when={props.replyTo}>
      {(reply) => (
        <div class="reply-preview">
          <span class="nf-icon reply-preview-icon">{ICON_REPLY}</span>
          <div class="reply-preview-content">
            <span class="reply-preview-sender">{reply().senderName}</span>
            <span class="reply-preview-body">{reply().body.length > 80 ? reply().body.slice(0, 80) + "..." : reply().body}</span>
          </div>
          <button
            class="reply-preview-close"
            onClick={props.onDismiss}
            title="Cancel reply"
            aria-label="Cancel reply"
          >
            <span class="nf-icon" aria-hidden="true">{ICON_CLOSE}</span>
          </button>
        </div>
      )}
    </Show>
  );
};

export default ReplyPreview;

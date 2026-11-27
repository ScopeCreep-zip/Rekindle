import { type Component, Show } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";

interface ReplyPreviewProps {
  replyTo: { senderName: string; body: string } | null;
  onDismiss: () => void;
}

const ReplyPreview: Component<ReplyPreviewProps> = (props) => {
  return (
    <Show when={props.replyTo}>
      {(reply) => (
        <div class="reply-preview">
          <SkinGlyph name="reply" class="reply-preview-icon" />
          <div class="reply-preview-content">
            <span class="reply-preview-sender">{reply().senderName}</span>
            <span class="reply-preview-body">{reply().body.length > 80 ? reply().body.slice(0, 80) + "..." : reply().body}</span>
          </div>
          <button
            type="button"
            class="reply-preview-close"
            onClick={props.onDismiss}
            title="Cancel reply"
            aria-label="Cancel reply"
          >
            <SkinGlyph name="close" />
          </button>
        </div>
      )}
    </Show>
  );
};

export default ReplyPreview;

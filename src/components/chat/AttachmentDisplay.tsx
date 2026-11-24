import { Component, Show, createMemo, createSignal } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";
import type { MessageAttachment } from "../../stores/chat.store";
import {
  handleDownloadAttachment,
  handlePinAttachment,
  handleRevealAttachment,
} from "../../actions/community.actions";
import { hasPermission, MANAGE_COMMUNITY } from "../../ipc/permissions";
import { communityState } from "../../stores/community.store";
import { servedPermissions } from "../../utils/permissions";
import { formatSize } from "../../utils/formatting";
import type { GlyphName } from "../common/glyph_art";

interface AttachmentDisplayProps {
  communityId: string;
  channelId: string;
  attachment: MessageAttachment;
}

function pickIcon(mimeType: string): GlyphName {
  if (mimeType.startsWith("image/")) return "file-image";
  return "file";
}

const AttachmentDisplay: Component<AttachmentDisplayProps> = (props) => {
  const [downloading, setDownloading] = createSignal(false);

  const downloaded = createMemo(() => props.attachment.downloaded);
  const pinned = createMemo(() => {
    const community = communityState.communities[props.communityId];
    return community?.pinnedAttachments?.includes(props.attachment.attachmentId) ?? false;
  });

  // Pinning is a community-wide setting: the backend's base flags.
  const myPerms = createMemo((): bigint =>
    servedPermissions(communityState.communities[props.communityId]),
  );
  const canPin = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));

  async function handleDownloadClick(): Promise<void> {
    if (downloading()) return;
    setDownloading(true);
    try {
      await handleDownloadAttachment(
        props.communityId,
        props.channelId,
        props.attachment.attachmentId,
      );
    } finally {
      setDownloading(false);
    }
  }

  async function handleRevealClick(): Promise<void> {
    await handleRevealAttachment(
      props.communityId,
      props.channelId,
      props.attachment.attachmentId,
    );
  }

  async function handlePinClick(): Promise<void> {
    await handlePinAttachment(props.communityId, props.attachment.attachmentId, !pinned());
  }

  return (
    <div class="attachment-card">
      <SkinGlyph
        name={pickIcon(props.attachment.mimeType)}
        class={`attachment-card-icon ${downloaded() ? "attachment-card-icon-ready" : ""}`}
      />
      <div class="attachment-card-meta">
        <div class="attachment-card-filename">{props.attachment.filename}</div>
        <div class="attachment-card-info">
          {formatSize(props.attachment.totalSize)} · {props.attachment.mimeType}
        </div>
      </div>
      <Show
        when={downloaded()}
        fallback={
          <button
            class="attachment-card-btn"
            onClick={() => void handleDownloadClick()}
            disabled={downloading()}
            title="Download"
          >
            <SkinGlyph name="download" />
            <span>{downloading() ? "Downloading…" : "Download"}</span>
          </button>
        }
      >
        <button
          class="attachment-card-btn"
          onClick={() => void handleRevealClick()}
          title="Show in folder"
        >
          <SkinGlyph name="folder-open" />
          <span>Show in folder</span>
        </button>
      </Show>
      <Show when={canPin()}>
        <button
          class={`attachment-card-pin-btn ${pinned() ? "attachment-card-pin-btn-active" : ""}`}
          onClick={() => void handlePinClick()}
          title={pinned() ? "Unpin attachment" : "Pin attachment"}
        >
          <SkinGlyph name="pin" />
        </button>
      </Show>
    </div>
  );
};

export default AttachmentDisplay;

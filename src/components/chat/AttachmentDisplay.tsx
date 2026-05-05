import { Component, Show, createMemo, createSignal } from "solid-js";
import type { MessageAttachment } from "../../stores/chat.store";
import { handleDownloadAttachment, handlePinAttachment } from "../../handlers/community.handlers";
import { hasPermission, MANAGE_COMMUNITY } from "../../ipc/permissions";
import { communityState } from "../../stores/community.store";
import { calculateBasePermissions } from "../../utils/permissions";
import {
  ICON_DOWNLOAD,
  ICON_FILE,
  ICON_FILE_IMAGE,
  ICON_FOLDER_OPEN,
  ICON_PIN,
} from "../../icons";

interface AttachmentDisplayProps {
  communityId: string;
  channelId: string;
  attachment: MessageAttachment;
}

const KB = 1024;
const MB = 1024 * 1024;

function formatSize(bytes: number): string {
  if (bytes < KB) return `${bytes} B`;
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${(bytes / MB).toFixed(1)} MB`;
}

function pickIcon(mimeType: string): string {
  if (mimeType.startsWith("image/")) return ICON_FILE_IMAGE;
  return ICON_FILE;
}

const AttachmentDisplay: Component<AttachmentDisplayProps> = (props) => {
  const [downloading, setDownloading] = createSignal(false);

  const downloaded = createMemo(() => Boolean(props.attachment.localPath));
  const pinned = createMemo(() => {
    const community = communityState.communities[props.communityId];
    return community?.pinnedAttachments?.includes(props.attachment.attachmentId) ?? false;
  });

  const myPerms = createMemo((): bigint => {
    const community = communityState.communities[props.communityId];
    if (!community) return 0n;
    return calculateBasePermissions(community.myRoleIds, community.roles);
  });
  const canPin = createMemo(() => hasPermission(myPerms(), MANAGE_COMMUNITY));

  async function handleDownloadClick(): Promise<void> {
    if (downloading()) return;
    setDownloading(true);
    try {
      await handleDownloadAttachment(
        props.communityId,
        props.channelId,
        props.attachment.attachmentId,
        props.attachment.filename,
      );
    } finally {
      setDownloading(false);
    }
  }

  async function handleOpenClick(): Promise<void> {
    const path = props.attachment.localPath;
    if (!path) return;
    const { openPath } = await import("@tauri-apps/plugin-opener");
    await openPath(path);
  }

  async function handlePinClick(): Promise<void> {
    await handlePinAttachment(props.communityId, props.attachment.attachmentId, !pinned());
  }

  return (
    <div class="attachment-card">
      <span class={`nf-icon attachment-card-icon ${downloaded() ? "attachment-card-icon-ready" : ""}`}>
        {pickIcon(props.attachment.mimeType)}
      </span>
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
            <span class="nf-icon">{ICON_DOWNLOAD}</span>
            <span>{downloading() ? "Downloading…" : "Download"}</span>
          </button>
        }
      >
        <button class="attachment-card-btn" onClick={() => void handleOpenClick()} title="Open">
          <span class="nf-icon">{ICON_FOLDER_OPEN}</span>
          <span>Open</span>
        </button>
      </Show>
      <Show when={canPin()}>
        <button
          class={`attachment-card-pin-btn ${pinned() ? "attachment-card-pin-btn-active" : ""}`}
          onClick={() => void handlePinClick()}
          title={pinned() ? "Unpin attachment" : "Pin attachment"}
        >
          <span class="nf-icon">{ICON_PIN}</span>
        </button>
      </Show>
    </div>
  );
};

export default AttachmentDisplay;

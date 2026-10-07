import { createStore } from "solid-js/store";

// Architecture §28.8 — sender-fetched OpenGraph metadata broadcast via
// gossip. Keyed by `linkPreviewKey(channelId, messageId)`: the backend
// only forwards a preview whose sender wrote that message in that
// channel, and the key keeps it attached to exactly that message.

export interface LinkPreviewData {
  url: string;
  title?: string;
  description?: string;
  siteName?: string;
  fetchedAt: number;
}

export function linkPreviewKey(channelId: string, messageId: string): string {
  return `${channelId}:${messageId}`;
}

const [linkPreviews, setLinkPreviews] = createStore<Record<string, LinkPreviewData>>({});

export { linkPreviews, setLinkPreviews };

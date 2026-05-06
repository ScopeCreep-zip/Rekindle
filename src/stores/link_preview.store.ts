import { createStore } from "solid-js/store";

// Architecture §28.8 — sender-fetched OpenGraph metadata broadcast via
// gossip, keyed by `(communityId, channelId, messageId)`. Storing
// flatly keyed by messageId is sufficient because messageId is a
// random 16-byte UUID — collisions across channels/communities are
// astronomically unlikely.

export interface LinkPreviewData {
  url: string;
  title?: string;
  description?: string;
  imageUrl?: string;
  siteName?: string;
  fetchedAt: number;
}

const [linkPreviews, setLinkPreviews] = createStore<Record<string, LinkPreviewData>>({});

export { linkPreviews, setLinkPreviews };

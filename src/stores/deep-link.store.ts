import { createSignal } from "solid-js";
import type { DeepLinkRequest } from "../ipc/commands/types";

/// The OS deep link awaiting the user's consent in the buddy list. A pure
/// mirror of the backend's pending slot (`src-tauri/src/deep_links.rs`).
const [pendingDeepLink, setPendingDeepLink] = createSignal<DeepLinkRequest | null>(null);

export { pendingDeepLink, setPendingDeepLink };

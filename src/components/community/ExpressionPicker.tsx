import { Component, For, Show, createMemo } from "solid-js";

import { communityState } from "../../stores/community.store";
import { handleUploadEmoji } from "../../handlers/community.handlers";
import { calculateBasePermissions, CREATE_EXPRESSIONS, hasPermission, MANAGE_EXPRESSIONS } from "../../ipc/permissions";

interface ExpressionPickerProps {
  communityId: string;
  mode: "reaction" | "message";
  searchQuery: string;
  onSelect: (value: string) => void;
}

function expressionSelectionValue(mode: "reaction" | "message", expressionId: string, name: string): string {
  if (mode === "reaction") {
    return `custom:${expressionId}`;
  }
  return `:${name}:`;
}

function sanitizeExpressionName(fileName: string): string {
  const baseName = fileName.replace(/\.[^.]+$/, "");
  const cleaned = baseName.replace(/[^A-Za-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  return cleaned.slice(0, 32) || "emoji";
}

const ExpressionPicker: Component<ExpressionPickerProps> = (props) => {
  const community = createMemo(() => communityState.communities[props.communityId]);
  const expressions = createMemo(() => {
    const query = props.searchQuery.trim().toLowerCase();
    const all = (community()?.expressions ?? []).filter((expression) => expression.kind === "emoji");
    if (!query) {
      return all;
    }
    return all.filter((expression) => expression.name.toLowerCase().includes(query));
  });
  const canUpload = createMemo(() => {
    const current = community();
    if (!current) return false;
    const perms = calculateBasePermissions(current.myRoleIds, current.roles);
    return hasPermission(perms, CREATE_EXPRESSIONS) || hasPermission(perms, MANAGE_EXPRESSIONS);
  });

  async function handleUploadClick(): Promise<void> {
    if (!canUpload()) return;
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/png,image/webp,image/gif";
    input.style.display = "none";
    document.body.appendChild(input);
    input.onchange = async () => {
      const file = input.files?.[0];
      document.body.removeChild(input);
      if (!file) return;
      const arrayBuffer = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(arrayBuffer));
      const defaultName = sanitizeExpressionName(file.name);
      const animated = file.type === "image/gif";
      await handleUploadEmoji(props.communityId, defaultName, bytes, animated);
    };
    input.click();
  }

  return (
    <div class="expression-picker-section">
      <div class="emoji-picker-section-label expression-picker-header">
        <span>Community Emoji</span>
        <Show when={canUpload()}>
          <button class="expression-picker-upload-btn" onClick={() => void handleUploadClick()}>
            Upload
          </button>
        </Show>
      </div>
      <Show
        when={expressions().length > 0}
        fallback={<div class="emoji-picker-empty expression-picker-empty">No custom emoji yet</div>}
      >
        <div class="emoji-picker-grid expression-picker-grid">
          <For each={expressions()}>
            {(expression) => (
              <button
                class="emoji-picker-item expression-picker-item"
                title={`:${expression.name}:`}
                onClick={() => props.onSelect(expressionSelectionValue(props.mode, expression.id, expression.name))}
              >
                <Show
                  when={expression.inlineDataUrl}
                  fallback={<span class="expression-picker-fallback">:{expression.name}:</span>}
                >
                  <img
                    class="expression-picker-image"
                    src={expression.inlineDataUrl!}
                    alt={`:${expression.name}:`}
                  />
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default ExpressionPicker;

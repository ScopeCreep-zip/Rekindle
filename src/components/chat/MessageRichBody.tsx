import { Component, For, createMemo } from "solid-js";

import { communityState } from "../../stores/community.store";

interface MessageRichBodyProps {
  communityId?: string;
  body: string;
}

type Segment =
  | { type: "text"; value: string }
  | { type: "emoji"; name: string; src: string };

const TOKEN_RE = /:([A-Za-z0-9_]{2,32}):/g;

const MessageRichBody: Component<MessageRichBodyProps> = (props) => {
  const segments = createMemo<Segment[]>(() => {
    if (!props.communityId) {
      return [{ type: "text", value: props.body }];
    }

    const expressions = new Map(
      (communityState.communities[props.communityId]?.expressions ?? [])
        .filter((expression) => expression.kind === "emoji" && expression.inlineDataUrl)
        .map((expression) => [expression.name, expression.inlineDataUrl!] as const),
    );

    const built: Segment[] = [];
    let lastIndex = 0;
    for (const match of props.body.matchAll(TOKEN_RE)) {
      const fullMatch = match[0];
      const name = match[1];
      const index = match.index ?? 0;
      const src = expressions.get(name);
      if (!src) {
        continue;
      }
      if (index > lastIndex) {
        built.push({ type: "text", value: props.body.slice(lastIndex, index) });
      }
      built.push({ type: "emoji", name, src });
      lastIndex = index + fullMatch.length;
    }
    if (lastIndex < props.body.length) {
      built.push({ type: "text", value: props.body.slice(lastIndex) });
    }
    return built.length > 0 ? built : [{ type: "text", value: props.body }];
  });

  return (
    <div class="chat-message-body">
      <For each={segments()}>
        {(segment) =>
          segment.type === "text" ? (
            <span>{segment.value}</span>
          ) : (
            <img
              class="chat-inline-expression"
              src={segment.src}
              alt={`:${segment.name}:`}
              title={`:${segment.name}:`}
            />
          )
        }
      </For>
    </div>
  );
};

export default MessageRichBody;

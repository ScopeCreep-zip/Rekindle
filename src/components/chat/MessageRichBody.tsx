import { Component, For, createMemo } from "solid-js";

import { communityState } from "../../stores/community.store";

interface MessageRichBodyProps {
  communityId?: string;
  body: string;
}

type Segment =
  | { type: "text"; value: string }
  | { type: "emoji"; name: string; src: string }
  | { type: "mention"; raw: string; kind: "everyone" | "here" | "role" | "user"; matchesMe: boolean };

const EMOJI_RE = /:([A-Za-z0-9_]{2,32}):/g;

/// Architecture §28.5 + §32 Week 18: parse `@everyone`, `@here`,
/// `@role-name`, and `@display-name` mentions out of the message body.
/// Mirrors the Rust parser in `services/community/mentions.rs` so the
/// same token-boundary rules apply (no false positives on emails).
const MENTION_RE = /(^|[\s\p{P}])@([A-Za-z0-9_-]+)/gu;

interface ResolvedMention {
  start: number;
  length: number;
  raw: string;
  kind: "everyone" | "here" | "role" | "user";
  matchesMe: boolean;
}

const MessageRichBody: Component<MessageRichBodyProps> = (props) => {
  const segments = createMemo<Segment[]>(() => {
    if (!props.communityId) {
      return [{ type: "text", value: props.body }];
    }

    const community = communityState.communities[props.communityId];
    const expressions = new Map(
      (community?.expressions ?? [])
        .filter((expression) => expression.kind === "emoji" && expression.inlineDataUrl)
        .map((expression) => [expression.name, expression.inlineDataUrl!] as const),
    );

    const myRoleNames = new Set<string>(
      (community?.myRoleIds ?? [])
        .map((id) => community?.roles?.find((r) => r.id === id)?.name?.toLowerCase())
        .filter((name): name is string => typeof name === "string"),
    );
    const myDisplayName =
      community?.members?.find((m) => m.pseudonymKey === community?.myPseudonymKey)?.displayName?.toLowerCase()
      ?? null;

    const mentions = collectMentions(props.body, community, myRoleNames, myDisplayName);
    const emojiSpans = collectEmojiSpans(props.body, expressions);

    return interleaveSpans(props.body, mentions, emojiSpans);
  });

  return (
    <div class="chat-message-body">
      <For each={segments()}>
        {(segment) => {
          if (segment.type === "text") {
            return <span>{segment.value}</span>;
          }
          if (segment.type === "emoji") {
            return (
              <img
                class="chat-inline-expression"
                src={segment.src}
                alt={`:${segment.name}:`}
                title={`:${segment.name}:`}
              />
            );
          }
          // mention
          return (
            <span
              class={`chat-mention chat-mention-${segment.kind}${segment.matchesMe ? " chat-mention-self" : ""}`}
              title={mentionTitle(segment.kind)}
            >
              {segment.raw}
            </span>
          );
        }}
      </For>
    </div>
  );
};

function mentionTitle(kind: "everyone" | "here" | "role" | "user"): string {
  switch (kind) {
    case "everyone":
      return "@everyone — pings every member";
    case "here":
      return "@here — pings online members";
    case "role":
      return "Role mention";
    case "user":
      return "User mention";
  }
}

function collectMentions(
  body: string,
  community:
    | { roles?: { name: string; mentionable: boolean }[]; members?: { displayName: string }[] }
    | undefined,
  myRoleNames: Set<string>,
  myDisplayName: string | null,
): ResolvedMention[] {
  const out: ResolvedMention[] = [];
  for (const match of body.matchAll(MENTION_RE)) {
    const prefix = match[1] ?? "";
    const token = match[2];
    if (!token) continue;
    const startWithPrefix = match.index ?? 0;
    const start = startWithPrefix + prefix.length;
    const length = token.length + 1; // include leading '@'
    const raw = body.slice(start, start + length);
    const lower = token.toLowerCase();

    let kind: "everyone" | "here" | "role" | "user";
    let matchesMe = false;
    if (lower === "everyone") {
      kind = "everyone";
      matchesMe = true;
    } else if (lower === "here") {
      kind = "here";
      matchesMe = true;
    } else {
      const matchedRole = community?.roles?.find(
        (r) => r.name.toLowerCase() === lower && r.mentionable,
      );
      if (matchedRole) {
        kind = "role";
        matchesMe = myRoleNames.has(lower);
      } else if (community?.members?.some((m) => m.displayName.toLowerCase() === lower)) {
        kind = "user";
        matchesMe = myDisplayName !== null && lower === myDisplayName;
      } else {
        // Unresolved — treat as plain text by skipping.
        continue;
      }
    }
    out.push({ start, length, raw, kind, matchesMe });
  }
  return out;
}

function collectEmojiSpans(body: string, expressions: Map<string, string>):
  { start: number; length: number; name: string; src: string }[] {
  const out: { start: number; length: number; name: string; src: string }[] = [];
  for (const match of body.matchAll(EMOJI_RE)) {
    const fullMatch = match[0];
    const name = match[1];
    const index = match.index ?? 0;
    const src = expressions.get(name);
    if (!src) continue;
    out.push({ start: index, length: fullMatch.length, name, src });
  }
  return out;
}

function interleaveSpans(
  body: string,
  mentions: ResolvedMention[],
  emojis: { start: number; length: number; name: string; src: string }[],
): Segment[] {
  type Span =
    | { kind: "mention"; data: ResolvedMention }
    | { kind: "emoji"; data: { start: number; length: number; name: string; src: string } };
  const all: Span[] = [
    ...mentions.map((m) => ({ kind: "mention", data: m } as Span)),
    ...emojis.map((e) => ({ kind: "emoji", data: e } as Span)),
  ];
  all.sort((a, b) => a.data.start - b.data.start);

  const out: Segment[] = [];
  let cursor = 0;
  for (const span of all) {
    if (span.data.start < cursor) continue; // overlap; skip the later one
    if (span.data.start > cursor) {
      out.push({ type: "text", value: body.slice(cursor, span.data.start) });
    }
    if (span.kind === "mention") {
      out.push({
        type: "mention",
        raw: span.data.raw,
        kind: span.data.kind,
        matchesMe: span.data.matchesMe,
      });
    } else {
      out.push({ type: "emoji", name: span.data.name, src: span.data.src });
    }
    cursor = span.data.start + span.data.length;
  }
  if (cursor < body.length) {
    out.push({ type: "text", value: body.slice(cursor) });
  }
  return out.length > 0 ? out : [{ type: "text", value: body }];
}

export default MessageRichBody;

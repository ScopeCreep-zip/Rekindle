import { createMemo, createSignal, type Accessor, type Setter } from "solid-js";
import { communityState } from "../../../stores/community.store";
import { calculateBasePermissions, hasPermission, MENTION_EVERYONE } from "../../../ipc/permissions";

// Architecture §28.5 — mentions: `@user`, `@role`, `@everyone`,
// `@here`. Backend resolves by display-name (members) or role-name
// (mentionable roles), with permission gating in
// `services/community/mentions.rs::validate_sender_permissions`.
export interface MentionCandidate {
  /** Token to splice into the message body, e.g. "alice" or "everyone". */
  token: string;
  /** Display label in the picker. */
  label: string;
  /** "member" / "role" / "special" — categorizes the picker rows. */
  kind: "member" | "role" | "special";
}

interface MentionAutocompleteArgs {
  communityId: () => string | undefined;
  body: Accessor<string>;
  setBody: Setter<string>;
  getTextarea: () => HTMLTextAreaElement | undefined;
}

export function useMentionAutocomplete(args: MentionAutocompleteArgs) {
  // `mentionQuery == null` means no active picker (last keystroke wasn't
  // preceded by a `@` token).
  const [mentionQuery, setMentionQuery] = createSignal<string | null>(null);
  const [mentionSelected, setMentionSelected] = createSignal(0);

  // Build the candidate list once per query change. Limited to 8 rows so
  // the popover never overruns the input.
  const mentionCandidates = createMemo<MentionCandidate[]>(() => {
    const q = mentionQuery();
    const communityId = args.communityId();
    if (q == null || !communityId) return [];
    const community = communityState.communities[communityId];
    if (!community) return [];
    const lc = q.toLowerCase();
    const matches = (text: string): boolean =>
      lc.length === 0 || text.toLowerCase().startsWith(lc);

    const candidates: MentionCandidate[] = [];

    // Architecture §17.2 + §28.5 — @everyone / @here gate locally;
    // sender's MENTION_EVERYONE bit determines whether the receiver
    // actually escalates the notification, but we surface them in the
    // picker for everyone (the literal text always renders).
    const perms = calculateBasePermissions(community.myRoleIds, community.roles);
    const canMentionEveryone = hasPermission(perms, MENTION_EVERYONE);
    if (canMentionEveryone) {
      if (matches("everyone")) {
        candidates.push({ token: "everyone", label: "@everyone — every member", kind: "special" });
      }
      if (matches("here")) {
        candidates.push({ token: "here", label: "@here — every online member", kind: "special" });
      }
    }

    // @role mentions — mentionable flag gates discovery.
    for (const role of community.roles ?? []) {
      if (!role.mentionable) continue;
      if (matches(role.name)) {
        candidates.push({ token: role.name, label: `@${role.name} (role)`, kind: "role" });
        if (candidates.length >= 8) break;
      }
    }

    // @member mentions — display name lookup.
    if (candidates.length < 8) {
      for (const member of community.members ?? []) {
        if (matches(member.displayName)) {
          candidates.push({ token: member.displayName, label: `@${member.displayName}`, kind: "member" });
          if (candidates.length >= 8) break;
        }
      }
    }

    return candidates;
  });

  function findMentionContext(value: string, caretIndex: number): string | null {
    // Look back from caret for the start of the active mention token. A
    // mention starts at `@` preceded by start-of-input, whitespace, or
    // punctuation — anything else means the user typed an email or similar.
    let i = caretIndex - 1;
    while (i >= 0) {
      const ch = value[i];
      if (ch === "@") {
        const prev = i > 0 ? value[i - 1] : "";
        if (i === 0 || /[\s.,;:!?(){}\[\]]/.test(prev)) {
          // Reject if there's a space between @ and caret — that means the
          // user has moved past a completed mention.
          const slice = value.slice(i + 1, caretIndex);
          if (/[\s]/.test(slice)) return null;
          return slice;
        }
        return null;
      }
      if (/[\s\n]/.test(ch)) return null;
      i -= 1;
    }
    return null;
  }

  function applyMention(candidate: MentionCandidate): void {
    const ta = args.getTextarea();
    if (!ta) return;
    const value = args.body();
    const caret = ta.selectionStart ?? value.length;
    // Walk back to the `@` that opened the picker.
    let start = caret - 1;
    while (start >= 0 && value[start] !== "@") start -= 1;
    if (start < 0) return;
    const before = value.slice(0, start);
    const after = value.slice(caret);
    const replacement = `@${candidate.token} `;
    const next = `${before}${replacement}${after}`;
    args.setBody(next);
    setMentionQuery(null);
    // Restore caret position right after the inserted token + space.
    queueMicrotask(() => {
      const newCaret = start + replacement.length;
      ta.setSelectionRange(newCaret, newCaret);
      ta.focus();
    });
  }

  /** Recompute the active mention query from the new caret position. */
  function recomputeOnInput(value: string, caret: number): void {
    setMentionQuery(findMentionContext(value, caret));
    setMentionSelected(0);
  }

  /** The picker absorbs ArrowUp/Down/Enter/Tab/Escape when open. Returns
   *  true when the key was consumed so the caller stops processing it. */
  function tryConsumeKeyDown(e: KeyboardEvent): boolean {
    if (mentionQuery() === null) return false;
    const candidates = mentionCandidates();
    if (e.key === "Escape") {
      e.preventDefault();
      setMentionQuery(null);
      return true;
    }
    if (candidates.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setMentionSelected((i) => (i + 1) % candidates.length);
        return true;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setMentionSelected((i) => (i - 1 + candidates.length) % candidates.length);
        return true;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        applyMention(candidates[mentionSelected()]);
        return true;
      }
    }
    return false;
  }

  return {
    mentionQuery,
    mentionCandidates,
    mentionSelected,
    setMentionSelected,
    applyMention,
    recomputeOnInput,
    tryConsumeKeyDown,
  };
}

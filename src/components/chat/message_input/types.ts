// Props for MessageInput, in a leaf so its own hooks can name them.
//
// These lived in MessageInput.tsx, which imports `useSlowmode` from this
// directory — and `useSlowmode` takes the props, so it imported the type
// straight back out of the component. A type-only edge still closes a
// module cycle: the bundler resolves both modules, and either file
// growing a value import turns a documentation problem into a
// load-order bug.

export interface EditMode {
  messageId: string;
  body: string;
}

export interface MessageInputProps {
  /** The input's accessible name: which conversation it writes to. */
  label: string;
  communityId?: string;
  peerId: string;
  replyTo?: { senderName: string; body: string; messageId?: string } | null;
  editMode?: EditMode | null;
  /** Send the draft: each window gives its own path (a 1:1 chat's
   *  send_message, a group's group_dm_send, a channel's post), so the input
   *  reaches no window's send command itself. */
  onSend: (id: string, body: string, replyToId?: string) => void;
  onDismissReply?: () => void;
  onEditSave?: (messageId: string, newBody: string) => void;
  onEditCancel?: () => void;
  onTyping?: () => void;
  /** Each edit's draft, for a window that reports typing from it (a 1:1
   *  chat, a group). */
  onDraftChanged?: (body: string) => void;
  disabled?: boolean;
  disabledMessage?: string;
  /** Shows Xfire's Send button beside the field (the 1:1 and group chat
   *  windows); disabled while the draft is empty. */
  sendButton?: boolean;
  slowmodeSeconds?: number;
  bypassSlowmode?: boolean;
}

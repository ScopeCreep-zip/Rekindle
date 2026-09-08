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
  communityId?: string;
  peerId: string;
  replyTo?: { senderName: string; body: string; messageId?: string } | null;
  editMode?: EditMode | null;
  onSend?: (id: string, body: string, replyToId?: string) => void;
  onDismissReply?: () => void;
  onEditSave?: (messageId: string, newBody: string) => void;
  onEditCancel?: () => void;
  onTyping?: () => void;
  disabled?: boolean;
  disabledMessage?: string;
  slowmodeSeconds?: number;
  bypassSlowmode?: boolean;
}

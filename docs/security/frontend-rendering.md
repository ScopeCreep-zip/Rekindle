# Frontend Rendering — Sanitization Gates

This document is the contract for any code path that renders
attacker-controlled content in Rekindle's webview. Any feature that
adds a new render path **must** clear the gates below before merging.

The webview is a real browser engine (WebKit / WebView2 / WebKitGTK)
running JavaScript with full DOM access. Even a single missed
sanitization on peer-controlled content turns into stored XSS, and
from there into capability-bypass that lets the renderer call sensitive
Rust commands. The threat-model entry is
[`threat-model.md` §5b W1](threat-model.md). The runtime tests live
under [`../../e2e/security/`](../../e2e/security/).

## When this gate applies

A render path renders **peer-controlled** content if any of the
following is true:

- Content comes from another peer over the gossip mesh (chat
  messages, reactions, custom emoji, embeds).
- Content comes from an external network resource that a peer can
  influence (link-preview titles/descriptions/images, OpenGraph
  payloads).
- Content comes from a peer's profile (display name, bio,
  pronouns, custom status, avatar metadata).
- Content comes from a peer's governance write (community names,
  channel names, role names, ban reasons, audit-log entries).
- Content comes from a peer's deep-link (the `rekindle://` payload
  decoded from another user's invite).

If any of these is true, the gate applies. **No exceptions.**

## The four gates

Any PR that adds a peer-controlled render path must:

### Gate 1 — SolidJS interpolation only

The default render is SolidJS's `{value}` interpolation, which
escapes the four HTML metacharacters (`<`, `>`, `"`, `&`). Use it
unmodified for plain-text fields:

```tsx
<span class="message-author">{message.author_display_name}</span>
<p class="message-body">{message.content}</p>
```

**Forbidden:**

```tsx
<div innerHTML={message.content} />          // ❌ XSS
<div ref={(el) => { el.innerHTML = body; }}/>// ❌ XSS via ref
<div dangerouslySetInnerHTML={{__html: x}}/> // ❌ React-ism, also XSS
```

The Semgrep rule [`rekindle-no-inner-html`](../../.semgrep.yml)
catches all three patterns.

### Gate 2 — Markdown / HTML / SVG content goes through DOMPurify

If the feature genuinely needs to render formatted content (markdown
bodies, HTML embeds, SVG previews from peer URLs), it goes through a
sanitizer. The standard sanitizer is [DOMPurify](https://github.com/cure53/DOMPurify)
(Cure53, audited).

```tsx
import DOMPurify from "dompurify";
import { marked } from "marked";

const SANITIZE_CONFIG: DOMPurify.Config = {
  ALLOWED_TAGS: [
    "p", "br", "em", "strong", "code", "pre", "blockquote",
    "ul", "ol", "li", "h1", "h2", "h3",
    "a", "del", "s",
  ],
  ALLOWED_ATTR: ["href", "title", "lang"],
  ALLOWED_URI_REGEXP: /^(?:https?|rekindle):/i,
  ADD_ATTR: ["target", "rel"],
  FORBID_TAGS: ["script", "iframe", "object", "embed", "style", "form", "input", "img"],
  FORBID_ATTR: ["style", "onload", "onerror", "onclick", "onmouseover"],
  RETURN_TRUSTED_TYPE: false,
};

function renderMarkdown(raw: string): string {
  const html = marked(raw, { breaks: true, gfm: true });
  return DOMPurify.sanitize(html as string, SANITIZE_CONFIG);
}

// Usage — the sanitized output is the only thing innerHTML ever sees:
<div innerHTML={renderMarkdown(message.content)}
     // nosemgrep: rekindle-no-inner-html
/>
```

**Forbidden patterns even with DOMPurify:**

```tsx
// ❌ Pre-formatting an attacker string before sanitizing it can
//    create context-switching attacks. Always sanitize the FINAL
//    HTML, not the intermediate markdown source.
DOMPurify.sanitize(`<p>${attackerString}</p>`)

// ❌ Permissive ALLOWED_TAGS that include `script`, `iframe`,
//    `object`, `embed`, `style`, `form`, or `img` (data: URIs).
ALLOWED_TAGS: ["script", ...]

// ❌ Permissive ALLOWED_URI_REGEXP that allows `data:` or `javascript:`.
ALLOWED_URI_REGEXP: /.*/
```

The DOMPurify config above is the **canonical Rekindle sanitizer**.
Copy it; do not invent a new one per feature.

### Gate 3 — Playwright XSS suite extension

For every new peer-controlled render path, add a test in
[`../../e2e/security/xss.spec.ts`](../../e2e/security/xss.spec.ts)
that:

1. Mocks the IPC response that delivers the field with each of the
   `XSS_PAYLOADS` corpus entries.
2. Navigates to the page that renders the field.
3. Asserts `window.__rekindleXssTriggered === false`.
4. Asserts the payload appears as text (escaped) or is filtered.

The skipped tests at the bottom of `xss.spec.ts` are placeholders
for the four highest-priority pending features. **Un-skip them
when the corresponding feature ships.**

### Gate 4 — URL handling for peer-supplied links

Links provided by peers (link previews, profile homepages, embed
URLs) get three checks:

1. **Scheme allowlist.** Only `https://`, `http://` (with a warning),
   and `rekindle://` are allowed. Reject `javascript:`, `data:`,
   `file:`, `vbscript:`, custom schemes.
2. **External nav uses the opener plugin.** Never assign to
   `window.location` directly. Use
   `import { open } from "@tauri-apps/plugin-opener";` and let the
   OS default handler take over. The Semgrep rule
   `rekindle-no-unchecked-href-assignment` catches the unsafe form.
3. **Anchor `rel` attribute.** When a link is rendered, always set
   `rel="noopener noreferrer ugc"` and `target="_blank"`.

```tsx
import { open } from "@tauri-apps/plugin-opener";

function safeOpen(url: string): void {
  if (!/^(?:https?|rekindle):/i.test(url)) {
    return; // refused
  }
  void open(url);
}

<a
  href={url}
  target="_blank"
  rel="noopener noreferrer ugc"
  onClick={(e) => {
    e.preventDefault();
    safeOpen(url);
  }}
>
  {url}
</a>
```

## DevDependencies to add

These are not yet in `package.json` because the features they support
(markdown rendering, link previews, custom emoji) haven't shipped.
**Add them in the same PR as the first feature that needs them**:

```json
{
  "devDependencies": {
    "dompurify": "^3.2.0",
    "@types/dompurify": "^3.2.0"
  },
  "dependencies": {
    "marked": "^14.0.0"
  }
}
```

## Audited innerHTML exceptions

Every `innerHTML` usage in the codebase must:

1. Be SVG (or HTML) generated by a trusted Rust IPC command — never
   peer content.
2. Carry a `// SAFETY (XSS):` comment block explaining the trust
   precondition and what would break the safety property.
3. Carry a `// nosemgrep: rekindle-no-inner-html` directive on the
   exact line of the `innerHTML` attribute.

Current audited exceptions:

| File | Line | Why allowed |
|------|------|-------------|
| `src/components/settings/AddDeviceModal.tsx` | ~298 | Pairing QR SVG generated locally by `commands.generatePairingQrSvg()` (Rust); no peer-controlled input flows into the SVG. |

If you add a new exception, add a row to the table.

## Per-render-path checklist

When implementing a feature that renders peer content:

- [ ] **Identify every field rendered.** List them in the PR
      description. (Examples: message body, message author display
      name, reaction emoji name, link-preview title.)
- [ ] **Each field uses SolidJS interpolation** unless it explicitly
      needs formatting.
- [ ] **Formatted fields go through DOMPurify** with the canonical
      config above.
- [ ] **Link fields go through `safeOpen`** with scheme allowlist.
- [ ] **A test in `xss.spec.ts`** covers every field with the
      `XSS_PAYLOADS` corpus.
- [ ] **The PR description references this document** and confirms
      every gate.

The PR template
([`../../.github/PULL_REQUEST_TEMPLATE.md`](../../.github/PULL_REQUEST_TEMPLATE.md))
has a "Security / privacy review" section that flags these.

## Related documentation

- [`threat-model.md`](threat-model.md) §5b "Frontend WebView attack surface"
- [`crypto-primitives.md`](crypto-primitives.md) §11 "What we do not use" (no frontend crypto)
- [`../../e2e/security/`](../../e2e/security/) — runtime test suite
- [`../../.semgrep.yml`](../../.semgrep.yml) — SAST rules
- [DOMPurify documentation](https://github.com/cure53/DOMPurify)
- [OWASP XSS Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)
- [OWASP DOM-based XSS Prevention Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/DOM_based_XSS_Prevention_Cheat_Sheet.html)

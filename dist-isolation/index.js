// Tauri isolation hook (plan E7.1, roadmap 3.3).
//
// Tauri runs this page in a sandboxed iframe of every window and hands it
// each IPC message the window sends, `{ cmd, callback, error, payload,
// options }`, before encrypting it for Rust (`tauri-utils` 2.10.1
// `src/pattern/isolation.js`, `payloadHandler`). Whatever this hook returns
// is what Rust receives; a throw means the message is never sent.
//
// A message passes only when `cmd` is one of the registered commands
// (`commands.js`, generated from `src-tauri/src/invoke.rs` and the capability
// grants by `cargo xtask gen-isolation-commands`) and its payload is a plain
// object. The one exception is Tauri's own channel fetch, which it invokes
// with a `null` payload (`tauri` 2.12.1 `src/ipc/channel.rs:319`). The message
// is returned unchanged.

const CHANNEL_FETCH = "plugin:__TAURI_CHANNEL__|fetch";

function isPlainObject(value) {
  return (
    typeof value === "object" &&
    value !== null &&
    Object.getPrototypeOf(value) === Object.prototype
  );
}

window.__TAURI_ISOLATION_HOOK__ = (message) => {
  const cmd = message.cmd;
  if (typeof cmd !== "string" || !REKINDLE_ISOLATION_COMMANDS.includes(cmd)) {
    throw new Error("isolation: refused an unregistered command");
  }
  const payloadOk =
    isPlainObject(message.payload) ||
    (cmd === CHANNEL_FETCH && message.payload === null);
  if (!payloadOk) {
    throw new Error("isolation: refused a payload that is not a plain object");
  }
  return message;
};

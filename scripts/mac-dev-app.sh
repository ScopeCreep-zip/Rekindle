#!/usr/bin/env bash
# macOS dev mode WITH camera: wrap the dev binary in a minimal .app.
#
# WebKit's GPU helper process resolves its camera sandbox extension
# against the host app's BUNDLE identity — a bare `pnpm tauri dev`
# binary has none, so `getUserMedia({video})` always fails with
# "Could not create a 'com.apple.webkit.camera' sandbox extension" /
# "No AVVideoCaptureSource device" no matter what TCC grants exist
# (tauri-apps/tauri#11951: fails in dev, works in the built app).
# This script builds the SAME dev binary `tauri dev` runs (it loads
# the Vite dev server at devUrl, so frontend hot-reload still works),
# wraps it in a one-file .app shell, and launches it via
# LaunchServices so macOS attributes camera access to the bundle.
#
# Usage:  ./scripts/mac-dev-app.sh
#   - starts the Vite dev server if :1430 isn't already listening
#   - rebuilds the debug binary (re-run the script after Rust changes;
#     frontend changes hot-reload as usual)
#   - first camera use prompts for "Rekindle Dev" — accept it
#   - a previously-declined prompt: tccutil reset Camera com.rekindle.app.dev
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/target/debug/dev-bundle/Rekindle Dev.app"
VITE_LOG="${TMPDIR:-/tmp}/rekindle-vite-dev.log"

# Rekindle's dev server runs on :1430 (NOT the Tauri scaffold default
# 1420), so it coexists with other Tauri/Vite projects on this machine
# instead of colliding — a collision on 1420 silently loaded a different
# app's frontend into the Rekindle shell. Probe via the system resolver
# (Vite binds the IPv6 loopback [::1], so a raw 127.0.0.1 check can report
# DOWN while the server is fine).
#   vite_up     — is anything answering on :1430?
#   is_rekindle — and is it REKINDLE's server (title marker), not another
#                 app squatting the port? (guards the silent-wrong-app bug)
vite_up() {
    curl -s -o /dev/null --max-time 2 "http://localhost:1430/" 2>/dev/null
}
is_rekindle() {
    curl -s --max-time 2 "http://localhost:1430/" 2>/dev/null | grep -q "<title>Rekindle</title>"
}

# 1. Frontend dev server (devUrl http://localhost:1430).
if vite_up; then
    if ! is_rekindle; then
        echo "✗ port 1430 is serving a DIFFERENT app, not Rekindle." >&2
        echo "  Free it ('lsof -ti:1430 | xargs kill') or change Rekindle's" >&2
        echo "  dev port (vite.config.ts + tauri.conf.json devUrl)." >&2
        exit 1
    fi
    echo "→ reusing the Rekindle Vite dev server already on :1430"
else
    echo "→ starting Vite dev server (log: $VITE_LOG)"
    (cd "$ROOT" && nohup pnpm dev >"$VITE_LOG" 2>&1 &)
    for _ in $(seq 1 120); do
        is_rekindle && break
        sleep 0.5
    done
    is_rekindle || {
        echo "✗ Rekindle Vite did not come up on :1430 — see $VITE_LOG" >&2
        exit 1
    }
fi

# 2. Dev binary — plain debug build, WITHOUT tauri/custom-protocol, so
#    the webview loads devUrl exactly like `pnpm tauri dev`.
echo "→ building dev binary"
(cd "$ROOT" && cargo build -p rekindle --bins)

# 3. Assemble the .app shell. The binary already embeds the usage
#    strings (tauri-build embed-plist of src-tauri/Info.plist); the
#    bundle plist adds the identity macOS needs for TCC + the WebKit
#    GPU-helper sandbox extension. Keep the usage strings in BOTH in
#    sync with src-tauri/Info.plist.
#
#    TCC pins camera/mic grants to the AD-HOC binary's cdhash — every
#    Rust rebuild invalidates them SILENTLY (WebKit then sees an empty
#    device list: getUserMedia fails OverconstrainedError, WebKit bug
#    177126 shape). Detect a changed binary and reset the grants so
#    the next capture re-prompts ONCE instead of failing silently.
#    (A stable codesigning identity would avoid the re-prompt; none is
#    configured on this machine.)
echo "→ assembling $APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
if [ -f "$APP_DIR/Contents/MacOS/rekindle" ] \
    && ! cmp -s "$ROOT/target/debug/rekindle" "$APP_DIR/Contents/MacOS/rekindle"; then
    echo "→ binary changed — resetting stale TCC camera/mic grants (one new prompt)"
    tccutil reset Camera com.rekindle.app.dev >/dev/null 2>&1 || true
    tccutil reset Microphone com.rekindle.app.dev >/dev/null 2>&1 || true
fi
cp "$ROOT/target/debug/rekindle" "$APP_DIR/Contents/MacOS/rekindle"
cat >"$APP_DIR/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.rekindle.app.dev</string>
	<key>CFBundleName</key>
	<string>Rekindle Dev</string>
	<key>CFBundleExecutable</key>
	<string>rekindle</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.0.1</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSCameraUsageDescription</key>
	<string>Rekindle uses your camera for video calls in voice channels and direct calls.</string>
	<key>NSMicrophoneUsageDescription</key>
	<string>Rekindle uses your microphone for voice chat in voice channels and direct calls.</string>
	<key>NSCameraUseContinuityCameraDeviceType</key>
	<true/>
</dict>
</plist>
PLIST

# 4. Launch through LaunchServices (NOT by exec'ing the binary — the
#    bundle attribution comes from how it's launched; running the
#    binary from a terminal would re-attribute TCC to the terminal).
#    `open --stdout/--stderr` captures the app's tracing output, which
#    otherwise goes nowhere; we then tail it live. A still-running
#    instance must exit first or `open` just activates it without the
#    new redirection.
APP_LOG="$ROOT/target/debug/dev-bundle/rekindle-dev.log"
if pgrep -f "dev-bundle/Rekindle Dev.app/Contents/MacOS/rekindle" >/dev/null; then
    echo "→ stopping the running Rekindle Dev instance"
    pkill -f "dev-bundle/Rekindle Dev.app/Contents/MacOS/rekindle" || true
    sleep 1
fi
echo "→ launching Rekindle Dev.app (log: $APP_LOG)"
: >"$APP_LOG"
open --stdout "$APP_LOG" --stderr "$APP_LOG" "$APP_DIR"
echo "✓ running — Rust changes need a re-run of this script; frontend hot-reloads"
echo "── live logs (Ctrl-C stops the tail, NOT the app) ──"
exec tail -f "$APP_LOG"

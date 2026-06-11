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
#   - starts the Vite dev server if :1420 isn't already listening
#   - rebuilds the debug binary (re-run the script after Rust changes;
#     frontend changes hot-reload as usual)
#   - first camera use prompts for "Rekindle Dev" — accept it
#   - a previously-declined prompt: tccutil reset Camera com.rekindle.app.dev
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/target/debug/dev-bundle/Rekindle Dev.app"
VITE_LOG="${TMPDIR:-/tmp}/rekindle-vite-dev.log"

# Probe the dev server the way the webview does: http://localhost:1420
# through the system resolver. Vite 6 binds the IPv6 loopback
# ([::1]:1420), so a raw 127.0.0.1 port check reports DOWN while the
# server is fine.
vite_up() {
    curl -s -o /dev/null --max-time 2 "http://localhost:1420/" 2>/dev/null
}

# 1. Frontend dev server (devUrl http://localhost:1420).
if ! vite_up; then
    echo "→ starting Vite dev server (log: $VITE_LOG)"
    (cd "$ROOT" && nohup pnpm dev >"$VITE_LOG" 2>&1 &)
    for _ in $(seq 1 120); do
        vite_up && break
        sleep 0.5
    done
    vite_up || {
        echo "✗ Vite did not come up on :1420 — see $VITE_LOG" >&2
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
echo "→ assembling $APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
cp "$ROOT/target/debug/rekindle" "$APP_DIR/Contents/MacOS/rekindle"
cat > "$APP_DIR/Contents/Info.plist" <<'PLIST'
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
#    bundle attribution comes from how it's launched).
echo "→ launching Rekindle Dev.app"
open "$APP_DIR"
echo "✓ running — Rust changes need a re-run of this script; frontend hot-reloads"

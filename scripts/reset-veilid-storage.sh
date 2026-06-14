#!/usr/bin/env bash
# Reset Rekindle's local VEILID node storage (table_store + protected_store).
#
# WHY THIS EXISTS
# --------------
# A veilid-core upgrade can change how veilid encrypts its on-disk table
# store. For example, veilid-core 0.5.3's ProtectedStore reaches for the OS
# keyring via a BLOCKING D-Bus call that panics under our async runtime, so
# we configure veilid to use file-based (insecure) protected storage instead
# (see crates/rekindle-protocol/src/node.rs). That changes the device
# encryption key, and veilid then refuses to open a table store written by an
# older build:
#
#   ERROR registry: Error initializing component 'RoutingTable':
#     ... has invalid encryption key, refusing to open database
#
# This script wipes ONLY veilid's regenerable node cache (its node identity,
# DHT record cache, and routes) so veilid recreates it cleanly on next start.
# It does NOT touch your identity vault, preferences, or chat history — log in
# afterward with the same account.
#
# Run this once per machine after a veilid upgrade that changes storage
# encryption (you'll know: the app gets stuck "connecting" and the dev log
# shows "invalid encryption key, refusing to open database").
#
# Usage:
#   scripts/reset-veilid-storage.sh        # preview + confirm
#   scripts/reset-veilid-storage.sh -y     # no prompt (for CI / scripted use)
#   pnpm reset:veilid                       # package.json alias
set -euo pipefail

APP_ID="com.rekindle.app"

ASSUME_YES=0
if [ "${1:-}" = "-y" ] || [ "${1:-}" = "--yes" ]; then
    ASSUME_YES=1
fi

# Resolve the per-OS application-data root.
case "$(uname -s)" in
    Linux)  DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}" ;;
    Darwin) DATA_HOME="$HOME/Library/Application Support" ;;
    *)
        echo "Unsupported OS: $(uname -s). This script supports Linux and macOS." >&2
        echo "On Windows, delete the 'table_store' and 'protected_store' folders" >&2
        echo "under %APPDATA%\\${APP_ID}\\veilid (and \\server) by hand." >&2
        exit 1
        ;;
esac

APP_DIR="$DATA_HOME/$APP_ID"     # Tauri app (what `pnpm dev:*` runs)
DAEMON_DIR="$DATA_HOME/rekindle" # standalone rekindle-node / rekindle-cli

# Veilid storage ROOTS (each holds the veilid sub-stores below). The Tauri app
# uses `veilid/`; its bundled server uses `server/`; the standalone daemon uses
# `<data>/rekindle/`.
ROOTS=(
    "$APP_DIR/veilid"
    "$APP_DIR/server"
    "$DAEMON_DIR/veilid"
    "$DAEMON_DIR/server"
)

# ONLY these veilid-owned sub-stores are removed under each root. Anything else
# living beside them (e.g. a sibling app DB) is left untouched.
SUBSTORES=(table_store protected_store block_store)

# Build the concrete delete list from paths that actually exist.
to_delete=()
for root in "${ROOTS[@]}"; do
    for sub in "${SUBSTORES[@]}"; do
        if [ -d "$root/$sub" ]; then
            to_delete+=("$root/$sub")
        fi
    done
done

if [ ${#to_delete[@]} -eq 0 ]; then
    echo "No veilid storage found under $DATA_HOME — nothing to reset."
    exit 0
fi

echo "Reset veilid node storage"
echo
echo "WILL DELETE (veilid node cache — regenerated on next start):"
printf '  %s\n' "${to_delete[@]}"
echo
echo "WILL KEEP (never touched):"
echo "  $APP_DIR/storage   ← your identity vault + salt"
echo "  $APP_DIR/*.db, preferences.json, chat history, webview caches"
echo

if [ "$ASSUME_YES" -ne 1 ]; then
    printf 'Proceed? [y/N] '
    read -r ans
    case "$ans" in
        y | Y | yes | YES) ;;
        *)
            echo "Aborted — nothing deleted."
            exit 1
            ;;
    esac
fi

for p in "${to_delete[@]}"; do
    rm -rf -- "$p"
    echo "removed $p"
done

echo
echo "Done. Start the app (e.g. 'pnpm dev:linux' or 'pnpm dev:mac') and log in"
echo "with your existing account — veilid will rebuild its node storage."

#!/bin/bash
# Rekindle identity initialization script.
# Called by rekindle-init.service on first boot.
# Reads the display name from /etc/rekindle/display-name (bind-mounted per node).
set -euo pipefail

DISPLAY_NAME_FILE="/etc/rekindle/display-name"

if [[ ! -f "${DISPLAY_NAME_FILE}" ]]; then
    echo "ERROR: ${DISPLAY_NAME_FILE} not found — cannot determine display name" >&2
    echo "This file should be bind-mounted by docker compose for each node." >&2
    exit 1
fi

DISPLAY_NAME=$(tr -d '\n' < "${DISPLAY_NAME_FILE}")

if [[ -z "${DISPLAY_NAME}" ]]; then
    echo "ERROR: ${DISPLAY_NAME_FILE} is empty" >&2
    exit 1
fi

echo "Initializing identity as '${DISPLAY_NAME}'..."

# Wait for the daemon to be fully reachable via IPC.
# The daemon sends sd_notify(READY=1) in Locked state, but the bus
# subscriber connects ~100ms later. Without the subscriber, requests
# are dropped with "daemon not connected to bus". We verify by checking
# that `rekindle status` returns a valid JSON response with a state field.
MAX_WAIT=30
for attempt in $(seq 1 "${MAX_WAIT}"); do
    if /usr/local/bin/rekindle status --format json 2>/dev/null | grep -q '"state"'; then
        echo "  Daemon reachable (attempt ${attempt}/${MAX_WAIT})"
        break
    fi
    if [[ "${attempt}" -eq "${MAX_WAIT}" ]]; then
        echo "ERROR: daemon not reachable after ${MAX_WAIT}s" >&2
        exit 1
    fi
    echo "  Waiting for daemon (attempt ${attempt}/${MAX_WAIT})..."
    sleep 1
done

# Allow an extra second for the subscriber to stabilize after the first
# successful status response — avoids the "not connected to bus" race.
sleep 2

exec /usr/local/bin/rekindle init --display-name "${DISPLAY_NAME}" --non-interactive

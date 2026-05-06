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

DISPLAY_NAME=$(cat "${DISPLAY_NAME_FILE}" | tr -d '\n')

if [[ -z "${DISPLAY_NAME}" ]]; then
    echo "ERROR: ${DISPLAY_NAME_FILE} is empty" >&2
    exit 1
fi

echo "Initializing identity as '${DISPLAY_NAME}'..."

# Wait for the daemon to be reachable via IPC.
# The daemon starts the bus server, then connects its own subscriber ~100ms later.
# Route allocation retry and network readiness are handled by the transport layer.
for attempt in $(seq 1 15); do
    if /usr/local/bin/rekindle status --format json 2>/dev/null | grep -q '"state"'; then
        break
    fi
    echo "  Waiting for daemon (attempt ${attempt}/15)..."
    sleep 1
done

exec /usr/local/bin/rekindle init --display-name "${DISPLAY_NAME}" --non-interactive

#!/bin/sh
# health-check.sh - Checks that a required service is running.
# Used by: health-check-job in kubjas.conf (uses random interval 30-60s)
# Exits 1 on failure so notify-failure fires.
SERVICE="${1:-nginx}"
if pgrep -x "$SERVICE" > /dev/null 2>&1; then
    echo "$SERVICE is running"
    exit 0
else
    echo "$SERVICE is NOT running" >&2
    exit 1
fi

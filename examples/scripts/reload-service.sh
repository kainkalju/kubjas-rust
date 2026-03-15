#!/bin/sh
# reload-service.sh - Reloads a service after its config file changes.
# Used by: config-watcher in kubjas.conf (interval = onchange + signal = HUP)
# When signal is set, kubjas sends the signal to the already-running process
# instead of starting a new one, so this script may never actually be executed
# directly — the signal does the work.
echo "Config reload triggered: $(date)"

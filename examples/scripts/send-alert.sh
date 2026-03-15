#!/bin/sh
# send-alert.sh - Sends a failure alert.
# Used by: alert-handler in kubjas.conf (interval = failure-message)
# Template variables %host%, %job%, %notify% are expanded by kubjas at runtime.
HOST="$1"
JOB="$2"
NOTIFY="$3"
echo "ALERT: job '$JOB' on '$HOST' reported '$NOTIFY' at $(date)" >> /var/log/kubjas-alerts.log
# Replace the line below with an actual notification command, e.g.:
#   mail -s "kubjas alert: $JOB failed on $HOST" admin@example.com <<< "$NOTIFY"

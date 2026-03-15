#!/bin/sh
# heartbeat.sh - Writes a timestamp to a heartbeat file every minute.
# Used by: heartbeat-job in kubjas.conf
date '+%Y-%m-%d %H:%M:%S' > /var/tmp/kubjas-heartbeat.txt
echo "Heartbeat OK"

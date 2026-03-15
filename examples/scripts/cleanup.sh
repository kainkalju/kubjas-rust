#!/bin/sh
# cleanup.sh - Deletes files older than 7 days from /var/tmp.
# Used by: cleanup-job in kubjas.conf (runs nightly at 02:00)
find /var/tmp -maxdepth 1 -type f -mtime +7 -delete
echo "Cleanup done: $(date)"

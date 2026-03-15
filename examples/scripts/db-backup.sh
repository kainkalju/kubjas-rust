#!/bin/sh
# db-backup.sh - Simulates a database backup (CPU-intensive, long-running).
# Used by: db-backup-job in kubjas.conf
# Runs with reduced priority (nice + ionice) so it does not starve other processes.
echo "Starting DB backup: $(date)"
sleep 5
echo "DB backup complete: $(date)"

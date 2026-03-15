#!/bin/sh
# index-rebuild.sh - Simulates a search index rebuild.
# Used by: index-rebuild-job in kubjas.conf
# Conflicts with db-backup-job so they never overlap.
echo "Rebuilding index: $(date)"
sleep 3
echo "Index rebuild complete: $(date)"

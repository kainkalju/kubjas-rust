#!/bin/sh
# step-one.sh - First stage in a pipeline.
# Used by: pipeline-step-one in kubjas.conf
# Notifies pipeline-step-two via notify-success when done.
echo "Step one running: $(date)"
sleep 1
echo "Step one done"

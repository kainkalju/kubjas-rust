#!/bin/sh
# step-two.sh - Second stage in a pipeline, triggered by step-one success.
# Used by: pipeline-step-two in kubjas.conf (interval = success-message)
# Template variable %job% names the job that sent the notify.
SOURCE_JOB="$1"
echo "Step two triggered by '$SOURCE_JOB': $(date)"
sleep 1
echo "Step two done"

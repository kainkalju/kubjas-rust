#!/bin/sh
# weekend-report.sh - Generates a weekly report every Saturday at 08:00.
# Used by: weekend-report-job in kubjas.conf (period filter)
echo "Weekly report generated: $(date)" >> /var/log/kubjas-reports.log

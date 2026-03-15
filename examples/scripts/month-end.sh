#!/bin/sh
# month-end.sh - Runs on the last day of the month at midnight.
# Used by: month-end-job in kubjas.conf
# Period is set to mday {28-31} with a secondary check in the script itself,
# since kubjas period cannot express "last day of month" directly.
TODAY=$(date +%d)
TOMORROW=$(date -d tomorrow +%d 2>/dev/null || date -v+1d +%d)
if [ "$TOMORROW" = "01" ]; then
    echo "Month-end processing: $(date)"
    # actual month-end logic here
else
    echo "Not the last day (day=$TODAY), skipping."
fi

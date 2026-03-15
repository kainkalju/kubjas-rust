#!/bin/sh
# process-upload.sh - Processes a file that was uploaded to /var/spool/uploads/.
# Used by: upload-processor in kubjas.conf (interval = onchange, watch dir)
# %notify% contains the path of the file that triggered the event.
CHANGED_FILE="$1"
echo "Processing: $CHANGED_FILE"
# Add real processing logic here, e.g. import into a database.

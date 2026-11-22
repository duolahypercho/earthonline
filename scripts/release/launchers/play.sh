#!/bin/sh
# Earth Online: walk and drive through San Francisco.
# Extra switches pass through: ./play.sh --beaver-house
cd "$(dirname "$0")" || exit 1
exec ./eo-app --play "$@"

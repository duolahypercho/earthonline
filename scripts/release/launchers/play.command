#!/bin/bash
# Earth Online: walk and drive through San Francisco.
# Double-click to start. From Terminal, extra switches pass through:
#   ./play.command --beaver-house
cd "$(dirname "$0")" || exit 1

# This build is not signed by Apple, so macOS flags the two programs as
# downloaded and refuses to start them. Running this script is the player's
# go-ahead; clear that flag from this folder's own programs, nothing else.
xattr -d com.apple.quarantine eo-app eo-server 2> /dev/null

./eo-app --play "$@"
status=$?
if [ "$status" -ne 0 ]; then
  echo
  echo "Earth Online stopped with an error ($status). The lines above say why; PLAYING.md lists common fixes."
  read -r -p "Press Return to close this window. " _
fi
exit "$status"

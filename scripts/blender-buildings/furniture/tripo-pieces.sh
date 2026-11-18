#!/usr/bin/env bash
# Text-prompted Tripo pieces for the furniture library (kinds the CC0 set lacks).
#   scripts/blender-buildings/furniture/tripo-pieces.sh <store dir> <kind> [<kind> ...]
# <store>/prompts.json holds the text prompt per kind (text only, never an image).
# Each kind: submit -> task id recorded in <kind>.task -> watch (retried) ->
# server decimate with that EXPLICIT id -> download into <kind>-lod/.
# A recorded task id is reused, so a rerun never pays for a generation twice.
set -uo pipefail
store="$1"; shift
cd "$store" || exit 2
watch() { for i in 1 2 3 4 5 6; do tripo --json task watch "$1" >/dev/null 2>&1 && return 0; sleep 20; done; return 1; }
status() { tripo --json task get "$1" 2>/dev/null | python3 -c "import json,sys;print(json.load(sys.stdin).get('status',''))" 2>/dev/null; }
for kind in "$@"; do
  prompt=$(python3 -c "import json,sys;print(json.load(open('prompts.json'))[sys.argv[1]])" "$kind")
  if [ ! -s "$kind.task" ]; then
    tripo --json generate text-to-model "$prompt" -p pbr=true -p texture=true -p texture_quality=detailed \
      --no-wait --name "interior-$kind" --yes --quiet --no-open \
      | python3 -c "import json,sys;d=json.load(sys.stdin);print(d.get('task_id') or d['task']['task_id'])" > "$kind.task" || { echo "$kind: submit failed"; rm -f "$kind.task"; continue; }
  fi
  gen=$(cat "$kind.task")
  watch "$gen"; st=$(status "$gen")
  [ "$st" = success ] || { echo "$kind: generation $gen is '$st'"; continue; }
  if [ ! -s "$kind.lod.task" ]; then
    tripo --json mesh decimate "$gen" --face-limit 1500 --no-wait --yes --quiet --no-open \
      | python3 -c "import json,sys;d=json.load(sys.stdin);print(d.get('task_id') or d['task']['task_id'])" > "$kind.lod.task" || { echo "$kind: decimate submit failed"; rm -f "$kind.lod.task"; continue; }
  fi
  lod=$(cat "$kind.lod.task")
  watch "$lod"; st=$(status "$lod")
  [ "$st" = success ] || { echo "$kind: decimate $lod is '$st'"; continue; }
  tripo task get "$lod" --download -o "$kind-lod" --yes --quiet --no-open >/dev/null 2>&1 || tripo --json task get "$lod" > "$kind-lod.json"
  echo "$kind: ok gen=$gen lod=$lod"
done
tripo balance

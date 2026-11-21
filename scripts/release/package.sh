#!/usr/bin/env bash
# Assemble a playable copy of the game from a release build and zip it.
#
#   scripts/release/package.sh <label> [out-dir]
#
# <label> names the platform in the archive name (windows-x64, macos-arm64,
# linux-x64); a label starting with "windows" packs eo-app.exe. Run it after
# `cargo build --release -p eo-app -p eo-server` in native/.
#
# The folder keeps the repository's layout (public/, native/<crate>/data,
# native/assets, the street-standards table), so the game finds its files
# from the executable's folder the way it finds them in a checkout. Only
# files tracked at HEAD are packed (git archive), never local scratch, caches
# or the optional off-repository assets.
set -euo pipefail

label=${1:?usage: package.sh <label> [out-dir]}
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${2:-$root/dist}
mkdir -p "$out"
out=$(cd "$out" && pwd)

version=$(git -C "$root" describe --tags --always)
name="EarthOnline-$version-$label"
dest="$out/$name"
ext=""
launcher="play.sh"
case "$label" in
  windows*) ext=".exe"; launcher="play.bat" ;;
  macos*) launcher="play.command" ;;
esac

bin="$root/native/target/release"
for exe in eo-app eo-server; do
  [[ -f "$bin/$exe$ext" ]] || { echo "missing $bin/$exe$ext: build it first" >&2; exit 1; }
done

rm -rf "$dest"
mkdir -p "$dest"

# The data the game reads at run time, and the licences and notices that
# travel with it (the map data is ODbL; see README.md and THIRD_PARTY_NOTICES.md).
git -C "$root" archive --format=tar HEAD -- \
  public \
  native/assets \
  ':(glob)native/*/data/**' \
  scripts/world-data/sources/california-street-standards.json \
  LICENSE LICENSES THIRD_PARTY_NOTICES.md README.md \
  | tar -x -C "$dest"

cp "$bin/eo-app$ext" "$bin/eo-server$ext" "$dest/"
cp "$root/Docs/PLAYING.md" "$dest/PLAYING.md"
cp "$root/scripts/release/launchers/$launcher" "$dest/$launcher"
chmod +x "$dest/eo-app$ext" "$dest/eo-server$ext" "$dest/$launcher"
git -C "$root" rev-parse HEAD > "$dest/BUILD-COMMIT.txt"

archive="$out/$name.zip"
rm -f "$archive"
if command -v zip > /dev/null; then
  (cd "$out" && zip -q -r -y "$archive" "$name")
elif command -v 7z > /dev/null; then
  (cd "$out" && 7z a -tzip -bd -bso0 "$archive" "$name")
else
  echo "neither zip nor 7z found to make $archive" >&2
  exit 1
fi
echo "$archive ($(du -h "$archive" | cut -f1))"

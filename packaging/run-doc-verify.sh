#!/usr/bin/env bash
# Run the verification commands of a documentation page against a release, so the page
# cannot drift from the release (AC-NFR-03). The page marks each command block that
# runs with `<!-- ci-run -->` on the line before the fence.
#
# Usage: run-doc-verify.sh <page.md> <version> <directory with the release files>
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: run-doc-verify.sh <page.md> <version> <directory>" >&2
  exit 2
fi
page="$1"
version="$2"
directory="$3"

script="$(mktemp)"
trap 'rm -f "$script"' EXIT

awk '
  /^[[:space:]]*<!-- ci-run -->[[:space:]]*$/ { armed = 1; next }
  armed && !inside && /^[[:space:]]*```/ { match($0, /^[[:space:]]*/); indent = RLENGTH; inside = 1; armed = 0; next }
  inside && /^[[:space:]]*```[[:space:]]*$/ { inside = 0; next }
  inside { print substr($0, indent + 1) }
' "$page" > "$script"

[ -s "$script" ] || { echo "error: $page has no command block marked ci-run" >&2; exit 1; }

# The page shows an example version. The run uses the version of the release.
sed -i "s/^VERSION=.*/VERSION=${version}/" "$script"
cd "$directory"
echo "Running the commands of $page:"
cat "$script"
bash -euo pipefail "$script"

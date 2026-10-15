#!/usr/bin/env bash
# Assemble an agent package from a built binary. The archive is reproducible (NFR-SEC-06):
# it depends on the file contents and on SOURCE_DATE_EPOCH only.
#
# Usage: assemble-agent-package.sh <binary> <os> <arch> <version> <out-dir>
# Prints the path of the archive.
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo "usage: assemble-agent-package.sh <binary> <os> <arch> <version> <out-dir>" >&2
  exit 2
fi
binary="$1"
os="$2"
arch="$3"
version="$4"
out="$5"
: "${SOURCE_DATE_EPOCH:?set SOURCE_DATE_EPOCH to the commit time}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
python="$(command -v python3 || command -v python)"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
name="$(basename "$binary")"
cp "$binary" "$stage/$name"
cp "$root/LICENSE" "$stage/LICENSE"
cp "$here/THIRD_PARTY_NOTICES" "$stage/THIRD_PARTY_NOTICES"
( cd "$stage" && sha256sum "$name" LICENSE THIRD_PARTY_NOTICES > SHA256SUMS )

mkdir -p "$out"
case "$os" in
  windows) archive="$out/restichub-agent-${version}-${os}-${arch}.zip" ;;
  *) archive="$out/restichub-agent-${version}-${os}-${arch}.tar.gz" ;;
esac
"$python" -I "$here/make-agent-archive.py" --directory "$stage" --out "$archive" \
  --epoch "$SOURCE_DATE_EPOCH" "$name" LICENSE THIRD_PARTY_NOTICES SHA256SUMS
echo "$archive"

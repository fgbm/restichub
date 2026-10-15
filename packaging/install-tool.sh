#!/usr/bin/env bash
# Download a release tool of a fixed version and install it only if the SHA-256 matches.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: install-tool.sh <url> <sha256> <name> <archive|binary>" >&2
  exit 2
fi
url="$1"
sha256="$2"
name="$3"
kind="$4"

dest="${RUNNER_TEMP:-/tmp}/tools/bin"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$dest"

curl --fail --silent --show-error --location --retry 3 --output "$work/download" "$url"
echo "$sha256  $work/download" | sha256sum --check --status ||
  { echo "error: the checksum of $name does not match" >&2; exit 1; }

case "$kind" in
  archive) tar -xzf "$work/download" -C "$work" "$name"; install -m 0755 "$work/$name" "$dest/$name" ;;
  binary) install -m 0755 "$work/download" "$dest/$name" ;;
  *) echo "error: unknown kind: $kind" >&2; exit 2 ;;
esac

if [ -n "${GITHUB_PATH:-}" ]; then
  echo "$dest" >> "$GITHUB_PATH"
fi

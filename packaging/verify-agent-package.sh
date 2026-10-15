#!/usr/bin/env bash
# Unpack an agent package and check its content (AC-IN-01). The package must not hold restic (ADR-0038).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: verify-agent-package.sh --package <file>

Unpack an agent package and check that it holds the agent, the license files and
the package hashes, and that it holds no restic.

Options:
  --package <file>  the tar.gz or zip package
  -h, --help        show this help
EOF
}

package=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --package) package="${2:?--package needs a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[ -n "$package" ] || { echo "error: --package is required" >&2; exit 2; }

extract_zip() {
  if command -v unzip >/dev/null 2>&1; then
    unzip -o -q "$1" -d "$2"
  else
    python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$1" "$2"
  fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

case "$package" in
  *.tar.gz) tar -xzf "$package" -C "$work" ;;
  *.zip) extract_zip "$package" "$work" ;;
  *) echo "error: unknown package format: $package" >&2; exit 2 ;;
esac

for name in restic restic.exe; do
  [ ! -e "$work/$name" ] || { echo "error: the package holds $name; the agent downloads restic (ADR-0038)" >&2; exit 1; }
done

agent=""
if [ -f "$work/restichub-agent" ]; then
  agent="$work/restichub-agent"
elif [ -f "$work/restichub-agent.exe" ]; then
  agent="$work/restichub-agent.exe"
fi
[ -n "$agent" ] || { echo "error: the package has no restichub-agent binary" >&2; exit 1; }

for name in LICENSE THIRD_PARTY_NOTICES SHA256SUMS; do
  [ -f "$work/$name" ] || { echo "error: the package has no $name" >&2; exit 1; }
done

grep -q 'downloads restic' "$work/THIRD_PARTY_NOTICES" ||
  { echo "error: THIRD_PARTY_NOTICES does not say that the agent downloads restic" >&2; exit 1; }

( cd "$work" && sha256sum -c SHA256SUMS >/dev/null )

echo "package ok: $package"

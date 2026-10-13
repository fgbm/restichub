#!/usr/bin/env bash
# Unpack an agent package and check its content and the restic hash (AC-IN-01).
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: verify-agent-package.sh --package <file> --expected-restic-sha256 <hash>

Unpack an agent package and check the content and the restic SHA-256.

Options:
  --package <file>                 the tar.gz or zip package
  --expected-restic-sha256 <hash>  the expected SHA-256 of restic
  -h, --help                       show this help
EOF
}

package=""
expected=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --package) package="${2:?--package needs a value}"; shift 2 ;;
    --expected-restic-sha256) expected="${2:?--expected-restic-sha256 needs a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[ -n "$package" ] || { echo "error: --package is required" >&2; exit 2; }
[ -n "$expected" ] || { echo "error: --expected-restic-sha256 is required" >&2; exit 2; }

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

restic=""
if [ -f "$work/restic" ]; then
  restic="$work/restic"
elif [ -f "$work/restic.exe" ]; then
  restic="$work/restic.exe"
fi
[ -n "$restic" ] || { echo "error: the package has no restic binary" >&2; exit 1; }

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

grep -q 'BSD 2-Clause' "$work/THIRD_PARTY_NOTICES" ||
  { echo "error: THIRD_PARTY_NOTICES has no restic BSD-2-Clause license text" >&2; exit 1; }

actual="$(sha256sum "$restic" | awk '{print $1}')"
if [ "$actual" != "$expected" ]; then
  echo "error: the restic SHA-256 differs from the pinned value" >&2
  echo "       expected $expected" >&2
  echo "       actual   $actual" >&2
  exit 1
fi

( cd "$work" && sha256sum -c SHA256SUMS >/dev/null )

echo "package ok: $package"

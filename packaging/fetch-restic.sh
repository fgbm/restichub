#!/usr/bin/env bash
# Fetch a pinned restic release, verify it against the signed SHA256SUMS of the
# restic project, and print the SHA-256 of the binary for RESTICHUB_RESTIC_SHA256.
# The pin and the signing key fingerprint are in restic.env (ADR-0011, ADR-0031).
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$script_dir/restic.env"
: "${RESTIC_VERSION:?restic.env does not set RESTIC_VERSION}"
: "${RESTIC_RELEASE_KEY_FINGERPRINT:?restic.env does not set RESTIC_RELEASE_KEY_FINGERPRINT}"

release_base="https://github.com/restic/restic/releases/download/v${RESTIC_VERSION}"
key_file="$script_dir/restic-release-key.asc"

usage() {
  cat <<'EOF'
Usage: fetch-restic.sh --os <linux|windows> --arch <amd64|arm64> --output <dir>

Download the pinned restic release, verify the SHA256SUMS signature and the
archive hash, unpack the binary into <dir>, and print the binary SHA-256.
The script also writes <dir>/restic.sha256 and <dir>/restic-LICENSE.

Options:
  --os <os>       linux or windows
  --arch <arch>   amd64 or arm64 (windows supports amd64 only)
  --output <dir>  the output directory
  -h, --help      show this help
EOF
}

os=""
arch=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --os) os="${2:?--os needs a value}"; shift 2 ;;
    --arch) arch="${2:?--arch needs a value}"; shift 2 ;;
    --output) output="${2:?--output needs a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$os" in
  linux|windows) ;;
  *) echo "error: --os must be linux or windows" >&2; exit 2 ;;
esac
case "$arch" in
  amd64|arm64) ;;
  *) echo "error: --arch must be amd64 or arm64" >&2; exit 2 ;;
esac
if [ "$os" = windows ] && [ "$arch" != amd64 ]; then
  echo "error: the pinned release has no windows $arch archive" >&2
  exit 2
fi
[ -n "$output" ] || { echo "error: --output is required" >&2; exit 2; }

for tool in curl gpg; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is required" >&2; exit 1; }
done
if [ "$os" = linux ]; then
  command -v bzip2 >/dev/null 2>&1 || { echo "error: bzip2 is required for a linux target" >&2; exit 1; }
fi

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

extract_zip() {
  if command -v unzip >/dev/null 2>&1; then
    unzip -o -q "$1" -d "$2"
  else
    python3 -c 'import sys, zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])' "$1" "$2"
  fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "restic ${RESTIC_VERSION} ${os}/${arch}" >&2
curl -fsSL "$release_base/SHA256SUMS" -o "$work/SHA256SUMS"
curl -fsSL "$release_base/SHA256SUMS.asc" -o "$work/SHA256SUMS.asc"

# A dedicated keyring keeps the run independent of the operator keyring.
export GNUPGHOME="$work/gnupg"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
gpg --batch --quiet --import "$key_file"

# The signature must come from the pinned key, not from any key in the key file.
status="$(gpg --batch --status-fd 1 --verify "$work/SHA256SUMS.asc" "$work/SHA256SUMS" 2>/dev/null || true)"
signed_by="$(printf '%s\n' "$status" | awk '/^\[GNUPG:\] VALIDSIG/{print $3; exit}')"
if [ "$signed_by" != "$RESTIC_RELEASE_KEY_FINGERPRINT" ]; then
  echo "error: SHA256SUMS is not signed by the pinned restic release key" >&2
  echo "       expected ${RESTIC_RELEASE_KEY_FINGERPRINT}, got ${signed_by:-none}" >&2
  exit 1
fi

case "$os" in
  linux) archive="restic_${RESTIC_VERSION}_${os}_${arch}.bz2" ;;
  windows) archive="restic_${RESTIC_VERSION}_${os}_${arch}.zip" ;;
esac
curl -fsSL "$release_base/$archive" -o "$work/$archive"

expected="$(awk -v name="$archive" '$2 == name {print $1}' "$work/SHA256SUMS")"
[ -n "$expected" ] || { echo "error: SHA256SUMS has no entry for $archive" >&2; exit 1; }
actual="$(sha256_of "$work/$archive")"
if [ "$actual" != "$expected" ]; then
  echo "error: $archive does not match SHA256SUMS" >&2
  echo "       expected $expected" >&2
  echo "       actual   $actual" >&2
  exit 1
fi

mkdir -p "$output"
case "$os" in
  linux)
    binary="restic"
    bzip2 -dc "$work/$archive" > "$output/$binary"
    chmod 0755 "$output/$binary"
    ;;
  windows)
    binary="restic.exe"
    extract_zip "$work/$archive" "$work/unpacked"
    source_binary="$(find "$work/unpacked" -type f -name 'restic*.exe' -print -quit)"
    [ -n "$source_binary" ] || { echo "error: the archive holds no restic executable" >&2; exit 1; }
    cp "$source_binary" "$output/$binary"
    ;;
esac

# The license text comes from the release source tree, listed in the same
# SHA256SUMS file (AC-IN-01).
source_archive="restic-${RESTIC_VERSION}.tar.gz"
curl -fsSL "$release_base/$source_archive" -o "$work/$source_archive"
source_expected="$(awk -v name="$source_archive" '$2 == name {print $1}' "$work/SHA256SUMS")"
[ -n "$source_expected" ] || { echo "error: SHA256SUMS has no entry for $source_archive" >&2; exit 1; }
if [ "$(sha256_of "$work/$source_archive")" != "$source_expected" ]; then
  echo "error: $source_archive does not match SHA256SUMS" >&2
  exit 1
fi
tar -xzf "$work/$source_archive" -C "$work" --strip-components=1 "restic-${RESTIC_VERSION}/LICENSE"
cp "$work/LICENSE" "$output/restic-LICENSE"

hash="$(sha256_of "$output/$binary")"
printf '%s\n' "$hash" > "$output/restic.sha256"
printf '%s\n' "$hash"

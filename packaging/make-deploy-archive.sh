#!/usr/bin/env bash
# Write the deploy archive and the published install.sh of a release (ADR-0056).
#
# Usage: make-deploy-archive.sh <version> <image reference with digest> <output directory>
#
# The archive is reproducible: sorted entries, the time SOURCE_DATE_EPOCH (default: the time
# of the last commit), owner and group 0, and gzip without a name and a time.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: make-deploy-archive.sh <version> <image@sha256:digest> <output directory>" >&2
  exit 2
fi
version=$1
image=$2
out=$3

semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
[[ "$version" =~ $semver ]] || { echo "error: $version is not a release version" >&2; exit 2; }
[[ "$image" =~ ^[a-z0-9.:/_-]+@sha256:[0-9a-f]{64}$ ]] ||
  { echo "error: $image is not an image reference with a digest" >&2; exit 2; }

root=$(cd "$(dirname "$0")/.." && pwd)
src=$root/deploy/reference
epoch=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}
name=restichub-deploy-$version

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -m 0755 "$work/$name"
dir=$work/$name

for file in compose.yaml Caddyfile README.md .env.example; do
  install -m 0644 "$src/$file" "$dir/$file"
done
install -m 0644 "$root/crates/restichub-db/sql/bootstrap-roles.sql" "$dir/bootstrap-roles.sql"
printf '%s\n' "$image" >"$dir/IMAGE"
chmod 0644 "$dir/IMAGE"

# The published script holds its version and no test seam, so a real run cannot reach the seam.
# A second marker pair or a lost last line would let sed cut the wrong lines or publish a
# script that never runs main.
seam_lines=$(grep -nx '# BEGIN TEST SEAM\|# END TEST SEAM' "$src/install.sh" | cut -d: -f2 | tr '\n' ' ')
[ "$seam_lines" = '# BEGIN TEST SEAM # END TEST SEAM ' ] ||
  { echo "error: install.sh must hold exactly one BEGIN and then one END marker of the test seam" >&2; exit 1; }
# shellcheck disable=SC2016
main_line='main "$@"'
[ "$(tail -n 1 "$src/install.sh")" = "$main_line" ] ||
  { echo "error: the last line of install.sh is not: $main_line" >&2; exit 1; }
sed -e '/^# BEGIN TEST SEAM$/,/^# END TEST SEAM$/d' \
  -e "s/^release_version=''\$/release_version='$version'/" \
  "$src/install.sh" >"$dir/install.sh"
chmod 0755 "$dir/install.sh"
grep -qx "release_version='$version'" "$dir/install.sh" ||
  { echo "error: the version line of install.sh is missing" >&2; exit 1; }
if grep -q 'RESTICHUB_TEST_\|TEST SEAM' "$dir/install.sh"; then
  echo "error: the published install.sh still holds the test seam" >&2
  exit 1
fi
[ "$(tail -n 1 "$dir/install.sh")" = "$main_line" ] ||
  { echo "error: the last line of the published install.sh is not: $main_line" >&2; exit 1; }
bash -n "$dir/install.sh"

mkdir -p "$out"
export LC_ALL=C
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner --format=gnu \
  -C "$work" -cf - "$name" | gzip -n -9 >"$out/$name.tar.gz"
install -m 0755 "$dir/install.sh" "$out/install.sh"
echo "$out/$name.tar.gz"
echo "$out/install.sh"

#!/usr/bin/env bash
# Run the release checks of the published install.sh with the keyless identity of the release
# (ADR-0056, T38). The CI smoke test runs the script through its test seam, so only this
# run checks the identity builder and the keyless verification of the script itself.
#
# Usage: verify-install-script.sh <directory with the release files> <version>
#
# The release files come from the directory instead of the release page, which is still a
# draft. cosign comes from its real URL and passes the SHA-256 check of the script.
# The sourced script defines and reads the shared variables.
# shellcheck disable=SC2034,SC2154
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: verify-install-script.sh <directory> <version>" >&2
  exit 2
fi
assets=$(cd "$1" && pwd)
expected_version=$2
script=$assets/install.sh

# shellcheck disable=SC2016
[ "$(tail -n 1 "$script")" = 'main "$@"' ] || { echo "error: the last line of $script is not main" >&2; exit 1; }
# Everything except the call of main: the functions and the pins of the published script.
# shellcheck source=/dev/null
source <(sed '$d' "$script")

[ "$release_version" = "$expected_version" ] || die "the published script holds the version $release_version"
version=$release_version
arch=amd64
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fetch() {
  case "$1" in
    "$releases_url/v$version/"*) install -m 0644 "$assets/${1##*/}" "$2" ;;
    *) curl --fail --silent --show-error --location --retry 3 --proto '=https' --output "$2" "$1" ;;
  esac
}

get_cosign
get_release
verify_image
set_check_args
echo "The published install.sh verified the archive and the image $image_ref with: ${check_args[*]}"

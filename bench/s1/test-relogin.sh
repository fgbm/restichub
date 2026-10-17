#!/usr/bin/env bash
# Checks that api() signs in again once after a 401. It replaces curl with a stub; no control plane runs.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mode=expire
# The counters are files because api() calls curl in subshells.
curl() {
	case "$*" in
	*auth/sign-in*) echo x >>"$tmp/signins"; return 0 ;;
	esac
	echo x >>"$tmp/calls"
	if [ "$mode" = always ] || { [ "$mode" = expire ] && [ ! -e "$tmp/signins" ]; }; then
		echo 'curl: (22) The requested URL returned error: 401' >&2
		return 22
	fi
	echo '{"ok":true}'
}
count() { if [ -e "$tmp/$1" ]; then wc -l <"$tmp/$1"; else echo 0; fi; }

out="$(api GET /api/v1/agents 2>/dev/null)"
[ "$out" = '{"ok":true}' ] && [ "$(count signins)" -eq 1 ] && [ "$(count calls)" -eq 2 ] || { echo "FAIL expired session"; exit 1; }

rm -f "$tmp/signins" "$tmp/calls"
mode=always
if api GET /api/v1/agents >/dev/null 2>&1; then echo "FAIL persistent 401 must fail"; exit 1; fi
[ "$(count signins)" -eq 1 ] && [ "$(count calls)" -eq 2 ] || { echo "FAIL retry is not bounded"; exit 1; }

echo ok

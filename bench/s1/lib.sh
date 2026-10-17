#!/usr/bin/env bash
# Shared helpers of the bench S1 scripts. Source this file; do not run it.

bench_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export STATE_DIR="${STATE_DIR:-$bench_dir/state}"
state_dir="$STATE_DIR"
env_file="$bench_dir/bench.env"
project="${COMPOSE_PROJECT_NAME:-restichub-bench-s1}"
export COMPOSE_PROJECT_NAME="$project"

web_port="${CP_WEB_PORT:-18080}"
agent_port="${CP_AGENT_PORT:-18443}"
web="http://127.0.0.1:$web_port"
admin_user="bench"
admin_password="bench-password-12345"
jar="$state_dir/cookies.txt"
csrf=(-H "x-restichub-csrf: 1")

dc() { docker compose --env-file "$env_file" -f "$bench_dir/compose.yaml" "$@"; }

log() { printf '%s %s\n' "$(date -u +%FT%TZ)" "$*" >&2; }

# api_once METHOD PATH [JSON]: one HTTP call as the signed-in administrator; prints the body. Exit code 22
# means an HTTP error status.
api_once() {
	local method="$1" path="$2" body="${3:-}"
	if [ -n "$body" ]; then
		curl -fsS -X "$method" "${csrf[@]}" -b "$jar" -H 'content-type: application/json' "$web$path" -d "$body"
	else
		curl -fsS -X "$method" "${csrf[@]}" -b "$jar" "$web$path"
	fi
}

# api METHOD PATH [JSON]: like api_once. After a 401 (the web session lives 12 hours) it signs in again and
# repeats the call once; a second failure is final.
api() {
	local out err rc=0
	err="$(mktemp)"
	out="$(api_once "$@" 2>"$err")" || rc=$?
	if [ "$rc" -eq 22 ] && grep -q ' 401' "$err"; then
		log "the session expired; signing in again"
		sign_in
		rc=0
		out="$(api_once "$@" 2>"$err")" || rc=$?
	fi
	if [ "$rc" -ne 0 ]; then cat "$err" >&2; fi
	rm -f "$err"
	if [ "$rc" -eq 0 ]; then printf '%s\n' "$out"; fi
	return "$rc"
}

sign_in() {
	curl -fsS "${csrf[@]}" -c "$jar" -H 'content-type: application/json' "$web/api/v1/auth/sign-in" \
		-d "{\"username\":\"$admin_user\",\"password\":\"$admin_password\"}" >/dev/null
}

# Lists every job of a query string, one JSON object per line, following the cursor.
all_jobs() {
	local query="${1:-}" cursor="" page
	while true; do
		page="$(api GET "/api/v1/jobs?limit=200${query:+&$query}${cursor:+&cursor=$cursor}")"
		printf '%s\n' "$page" | jq -c '.items[]'
		cursor="$(printf '%s' "$page" | jq -r '.next_cursor // empty')"
		[ -n "$cursor" ] || break
	done
}

# wait_for SECONDS DESCRIPTION COMMAND...
wait_for() {
	local seconds="$1" what="$2"
	shift 2
	local end=$((SECONDS + seconds))
	while [ "$SECONDS" -lt "$end" ]; do
		if "$@" >/dev/null 2>&1; then return 0; fi
		sleep 3
	done
	log "timeout after ${seconds}s: $what"
	return 1
}

# The process identifiers of the restic processes in an agent host. The agent starts restic from a
# verified file descriptor, so the process name is a number: the command line identifies it.
restic_pids() {
	docker exec "$1" sh -c 'for p in /proc/[0-9]*; do
		case "$(tr "\0" " " <"$p/cmdline" 2>/dev/null)" in
		/proc/self/fd/*) echo "${p#/proc/}" ;;
		esac
	done; true'
}

#!/usr/bin/env bash
# Helpers of the install smoke workflow (AC-IN-03). The script runs on a Linux and on a Windows
# runner (Git Bash). Each step of the workflow calls one command.
#
#   install-smoke.sh db-prepare   create the roles, the passwords and the schema
#   install-smoke.sh cp-up        start the control plane, create the administrator and the token
#   install-smoke.sh ca           write the installation CA that the listener presents
#   install-smoke.sh token FILE   write a new enrollment token to FILE
#   install-smoke.sh connected    wait until the control plane lists the agent as connected
#   install-smoke.sh canary-jobs  run ten jobs on three repositories whose passwords are canaries
#   install-smoke.sh cp-logs      print the log of the control plane
#
# The state lives in $SMOKE_DIR. The values are for a throw-away control plane on the loopback
# address only.

set -euo pipefail

smoke_dir="${SMOKE_DIR:?set SMOKE_DIR}"
web="http://127.0.0.1:8080"
listener_host="localhost"
listener_port="8443"
admin_password="smoke-password-12345"

csrf=(-H "x-restichub-csrf: 1")
jar="$smoke_dir/cookies.txt"

db_prepare() {
	# PGHOST, PGPORT, PGUSER and PGPASSWORD of an administrator come from the environment.
	local root
	root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
	psql -d postgres -v ON_ERROR_STOP=1 \
		-v app_role=restichub_app -v auth_role=restichub_auth \
		-v owner_role=restichub_owner -v runtime_role=restichub_runtime \
		-f "$root/crates/restichub-db/sql/bootstrap-roles.sql"
	psql -d postgres -v ON_ERROR_STOP=1 <<'SQL'
ALTER ROLE restichub_owner PASSWORD 'owner';
ALTER ROLE restichub_runtime PASSWORD 'runtime';
ALTER DATABASE restichub OWNER TO restichub_owner;
SQL
	RESTICHUB_DATABASE_MIGRATION_URL="postgres://restichub_owner@127.0.0.1:5432/restichub" \
		RESTICHUB_DATABASE_MIGRATION_PASSWORD=owner \
		"${CP_BIN:?set CP_BIN}" migrate
}

cp_up() {
	mkdir -p "$smoke_dir"
	export RESTICHUB_LISTEN_ADDR="127.0.0.1:8080"
	export RESTICHUB_DATABASE_URL="postgres://restichub_runtime@127.0.0.1:5432/restichub"
	export RESTICHUB_DATABASE_PASSWORD=runtime
	RESTICHUB_KEK="$(head -c 32 /dev/urandom | base64)"
	export RESTICHUB_KEK
	export RESTICHUB_SERVER_INSECURE_COOKIES=true
	export RESTICHUB_TENANCY_MODE=single
	export RESTICHUB_AGENT_LISTEN="127.0.0.1:$listener_port"
	export RESTICHUB_AGENT_ENDPOINT="https://$listener_host:$listener_port"
	export NO_COLOR=1
	"${CP_BIN:?set CP_BIN}" serve >"$smoke_dir/cp.log" 2>&1 &
	echo $! >"$smoke_dir/cp.pid"
	for _ in $(seq 1 60); do
		if curl -fsS "$web/readyz" >/dev/null 2>&1; then break; fi
		sleep 1
	done
	curl -fsS "$web/readyz" >/dev/null

	local setup_token
	setup_token="$(sed -n 's/.*setup_token=\([^ ]*\).*/\1/p' "$smoke_dir/cp.log" | head -n1 | tr -d '"\r')"
	[ -n "$setup_token" ] || { echo "no setup token in the control plane log" >&2; exit 1; }
	curl -fsS "${csrf[@]}" -H 'content-type: application/json' "$web/api/v1/setup" \
		-d "{\"setup_token\":\"$setup_token\",\"username\":\"smoke\",\"password\":\"$admin_password\",\"display_name\":\"Smoke\"}" \
		>/dev/null
	curl -fsS "${csrf[@]}" -c "$jar" -H 'content-type: application/json' "$web/api/v1/auth/sign-in" \
		-d "{\"username\":\"smoke\",\"password\":\"$admin_password\"}" >/dev/null
	token "$smoke_dir/enrollment-token"
}

token() {
	local file="${1:?the token file}"
	curl -fsS "${csrf[@]}" -b "$jar" -H 'content-type: application/json' "$web/api/v1/enrollment-tokens" \
		-d '{"ttl_seconds":3600}' | jq -r .token >"$file"
	[ -s "$file" ]
}

# The listener certificate comes from the installation CA, which no system store holds. The smoke
# job runs the control plane itself, so it takes the CA from the first handshake.
ca() {
	local openssl_bin="${OPENSSL:-openssl}"
	"$openssl_bin" s_client -showcerts -connect "$listener_host:$listener_port" \
		-servername "$listener_host" </dev/null 2>/dev/null |
		awk '/BEGIN CERTIFICATE/{n++} n>0{print > ("'"$smoke_dir"'/chain-" n ".pem")} /END CERTIFICATE/{}' 
	local count
	count="$(ls "$smoke_dir"/chain-*.pem | wc -l)"
	echo "the listener presented $count certificate(s)"
	if [ "$count" -ge 2 ]; then
		# Every certificate after the leaf is a CA of the installation.
		for file in "$smoke_dir"/chain-*.pem; do
			[ "$file" = "$smoke_dir/chain-1.pem" ] || cat "$file"
		done >"$smoke_dir/ca.pem"
	else
		cp "$smoke_dir/chain-1.pem" "$smoke_dir/ca.pem"
	fi
}

agent_json() {
	curl -fsS -b "$jar" "$web/api/v1/agents"
}

# Connected means a heartbeat updated last_seen_at after $1 (epoch seconds).
connected() {
	local since="${1:-0}" timeout="${2:-60}" body seen
	for _ in $(seq 1 "$timeout"); do
		body="$(agent_json || true)"
		seen="$(printf '%s' "$body" | jq -r '.items[0].last_seen_at // empty' 2>/dev/null || true)"
		if [ -n "$seen" ] && [ "$(date -u -d "$seen" +%s 2>/dev/null || echo 0)" -ge "$since" ]; then
			echo "agent connected: $(printf '%s' "$body" | jq -c '.items[0] | {hostname, status, last_seen_at, agent_version}')"
			return 0
		fi
		sleep 1
	done
	echo "the agent is not connected after ${timeout}s; last answer: $body" >&2
	return 1
}

wait_job() {
	local id="$1" status=""
	for _ in $(seq 1 120); do
		status="$(curl -fsS -b "$jar" "$web/api/v1/jobs/$id" | jq -r .status)"
		case "$status" in
		succeeded) return 0 ;;
		failed | expired) break ;;
		esac
		sleep 1
	done
	echo "job $id ended with status '$status'" >&2
	curl -fsS -b "$jar" "$web/api/v1/jobs/$id" >&2 || true
	return 1
}

# Right after a restart of the service, the control plane can still answer 409 agent_offline.
start_job() {
	local answer code
	for _ in $(seq 1 30); do
		answer="$(curl -sS "${csrf[@]}" -b "$jar" -H 'content-type: application/json' \
			-w '\n%{http_code}' "$web/api/v1/repositories/$1/$2" -d '{}')"
		code="${answer##*$'\n'}"
		if [ "$code" = 202 ]; then
			printf '%s' "${answer%$'\n'*}" | jq -r .id
			return 0
		fi
		[ "$code" = 409 ] || break
		sleep 1
	done
	echo "cannot start the $2 job: HTTP $code ${answer%$'\n'*}" >&2
	return 1
}

run_job() {
	local id
	id="$(start_job "$1" "$2")"
	wait_job "$id"
}

# The installed service runs ten jobs on three local repositories below $2 (AC-AG-03). The password
# of each repository is one line of the file $1. The password goes to curl on the standard input,
# so it is never a process argument.
canary_jobs() {
	local canaries="$1" root="$2" agent index=0 repository
	local repositories=()
	agent="$(agent_json | jq -r '.items[0].id')"
	while IFS= read -r canary; do
		index=$((index + 1))
		repository="$(printf '%s' "$canary" |
			jq -Rc --arg name "canary-$index" --arg path "$root/repo-$index" --arg agent "$agent" \
				'{name: $name, backend: {kind: "local", path: $path}, password: ., maintenance_agent_id: $agent}' |
			curl -fsS "${csrf[@]}" -b "$jar" -H 'content-type: application/json' \
				"$web/api/v1/repositories" --data-binary @- | jq -r .id)"
		repositories+=("$repository")
	done <"$canaries"
	[ "${#repositories[@]}" -eq 3 ] || { echo "three canaries are necessary" >&2; return 1; }
	for repository in "${repositories[@]}"; do
		run_job "$repository" init
		run_job "$repository" test-access
		run_job "$repository" test-access
	done
	run_job "${repositories[0]}" test-access
	echo "ten canary jobs succeeded"
}

case "${1:-}" in
db-prepare) db_prepare ;;
cp-up) cp_up ;;
ca) ca ;;
token) shift; token "$@" ;;
connected) shift; connected "$@" ;;
canary-jobs) shift; canary_jobs "$@" ;;
cp-logs) cat "$smoke_dir/cp.log" ;;
*) echo "unknown command: ${1:-}" >&2; exit 2 ;;
esac

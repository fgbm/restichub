#!/usr/bin/env bash
# Bench S1 control script: the control plane, PostgreSQL, one repository and the Linux agent hosts.
#
#   bench.sh init       write bench.env with random secrets (once)
#   bench.sh build      build the control plane image and the two agent images
#   bench.sh up         start the stack
#   bench.sh setup      create the administrator, the enrollment tokens, the repository, the
#                       profiles, the plans and the maintenance schedules
#   bench.sh start      init, build, up and setup in one go
#   bench.sh status     print the agents and the job counts
#   bench.sh logs [svc] print the logs of a service
#   bench.sh stop       stop the containers and keep the state
#   bench.sh reset      remove the containers, the volumes and the state directory
#
# Settings (environment): AGENTS_A, AGENTS_B (restic 0.18.1 and 0.19.1 hosts, default 10 each),
# PLAN_PERIOD_MIN (backup period in minutes, default 15), RETENTION_CRON, CHECK_CRON,
# NO_PLANS=1 (no repository, no plan: the idle host of AC-AG-02), COMPOSE_PROJECT_NAME,
# CP_WEB_PORT, CP_AGENT_PORT.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

repo_password="bench-repository-password"

random() { head -c 24 /dev/urandom | base64 | tr -d '/+=\n'; }

init() {
	if [ -e "$env_file" ]; then
		log "$env_file exists; keeping it"
		return
	fi
	{
		echo "POSTGRES_USER=postgres"
		echo "POSTGRES_PASSWORD=$(random)"
		echo "POSTGRES_DB=restichub"
		echo "RESTICHUB_OWNER_PASSWORD=$(random)"
		echo "RESTICHUB_RUNTIME_PASSWORD=$(random)"
		echo "RESTICHUB_KEK=$(head -c 32 /dev/urandom | base64)"
	} >"$env_file"
	chmod 600 "$env_file"
}

fetch_restic() {
	local version="$1" dir="$bench_dir/out/restic/$1" base file
	base="https://github.com/restic/restic/releases/download/v$version"
	mkdir -p "$dir"
	for file in "restic_${version}_linux_amd64.bz2" SHA256SUMS SHA256SUMS.asc; do
		[ -s "$dir/$file" ] || curl -fsSL -o "$dir/$file" "$base/$file"
	done
}

build() {
	fetch_restic "${RESTIC_A:-0.18.1}"
	fetch_restic "${RESTIC_B:-0.19.1}"
	export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
	DOCKER_BUILDKIT=1 dc build "$@"
}

up() {
	mkdir -p "$state_dir/tokens"
	chmod 777 "$state_dir/tokens"
	dc up -d
}

ready() { curl -fsS "$web/readyz" >/dev/null 2>&1; }

setup_admin() {
	local token
	if curl -fsS "$web/api/v1/setup/status" | jq -e '.setup_required == false' >/dev/null; then
		log "the setup is done; signing in"
	else
		token="$(dc logs cp 2>/dev/null | sed -n 's/.*setup_token=\([^ ]*\).*/\1/p' | head -n1 | tr -d '"\r')"
		[ -n "$token" ] || { log "no setup token in the control plane log"; return 1; }
		curl -fsS "${csrf[@]}" -H 'content-type: application/json' "$web/api/v1/setup" \
			-d "{\"setup_token\":\"$token\",\"username\":\"$admin_user\",\"password\":\"$admin_password\",\"display_name\":\"Bench\"}" >/dev/null
	fi
	sign_in
}

# The listener certificate comes from the installation CA that no system store holds. The agents
# take the CA of the chain that the listener presents at the first handshake.
write_ca() {
	local out="$state_dir/tokens/ca.pem" tmp
	tmp="$(mktemp -d)"
	openssl s_client -showcerts -connect "127.0.0.1:$agent_port" -servername cp </dev/null 2>/dev/null |
		awk -v dir="$tmp" '/BEGIN CERTIFICATE/{n++} n>0{print > (dir "/chain-" n ".pem")}'
	if [ "$(ls "$tmp" | wc -l)" -ge 2 ]; then
		for file in "$tmp"/chain-*.pem; do
			[ "$file" = "$tmp/chain-1.pem" ] || cat "$file"
		done >"$out.part"
	else
		cp "$tmp/chain-1.pem" "$out.part"
	fi
	rm -rf "$tmp"
	chmod 644 "$out.part"
	mv "$out.part" "$out"
}

write_tokens() {
	local count="$1" i token
	for i in $(seq 1 "$count"); do
		token="$(api POST /api/v1/enrollment-tokens '{"ttl_seconds":86400}' | jq -r .token)"
		printf '%s' "$token" >"$state_dir/tokens/token-$(printf '%03d' "$i").part"
		mv "$state_dir/tokens/token-$(printf '%03d' "$i").part" "$state_dir/tokens/token-$(printf '%03d' "$i")"
		chmod 666 "$state_dir/tokens/token-$(printf '%03d' "$i")"
	done
}

connected_agents() { api GET '/api/v1/agents?limit=200' | jq '[.items[] | select(.status == "online" or .status == "connected" or .status == "active")] | length'; }

setup() {
	local count="${AGENTS_A:-10}"
	count=$((count + ${AGENTS_B:-10}))
	local period="${PLAN_PERIOD_MIN:-15}"
	wait_for 180 "the control plane" ready
	setup_admin
	if [ ! -e "$state_dir/tokens/ca.pem" ]; then
		write_ca
		write_tokens "$count"
	fi
	log "waiting for $count agents"
	wait_for 600 "$count agents" bash -c "$(declare -f api); $(declare -f connected_agents); web='$web'; jar='$jar'; csrf=(-H 'x-restichub-csrf: 1'); [ \"\$(connected_agents)\" -ge $count ]"

	if [ "${NO_PLANS:-0}" = 1 ]; then
		log "no plans: the agents stay connected and idle"
		date -u +%FT%TZ >"$state_dir/setup-finished-at"
		return
	fi
	local agents first repo_id profile plan i offset minutes
	agents="$(api GET '/api/v1/agents?limit=200' | jq -r '.items | sort_by(.hostname) | .[].id')"
	first="$(printf '%s\n' "$agents" | head -n1)"
	repo_id="$(api GET /api/v1/repositories | jq -r '.items[]? | select(.name == "bench") | .id' | head -n1)"
	if [ -z "$repo_id" ]; then
		repo_id="$(api POST /api/v1/repositories "$(jq -nc --arg a "$first" --arg p "$repo_password" \
			'{name:"bench",backend:{kind:"rest",endpoint:"http://rest-server:8000/bench"},password:$p,maintenance_agent_id:$a}')" | jq -r '.id // .repository.id')"
	fi
	init_job="$(api POST "/api/v1/repositories/$repo_id/init" '{}' | jq -r .id)"
	wait_for 180 "the repository init" bash -c "$(declare -f api); web='$web'; jar='$jar'; csrf=(-H 'x-restichub-csrf: 1'); [ \"\$(api GET /api/v1/jobs/$init_job | jq -r .status)\" = succeeded ]"
	echo "$repo_id" >"$state_dir/repository-id"

	i=0
	for agent in $agents; do
		offset=$((i * period / count))
		minutes="$(for k in $(seq 0 $((60 / period - 1))); do printf '%s,' $((offset + k * period)); done)"
		minutes="${minutes%,}"
		profile="$(api POST /api/v1/profiles "$(jq -nc --arg n "p$(printf '%02d' "$i")" \
			'{name:$n,paths:{all:["/data"]},retention:{keep_last:6,keep_hourly:24,keep_daily:7}}')" | jq -r '.profile.id')"
		plan="$(api POST /api/v1/plans "$(jq -nc --arg p "$profile" --arg a "$agent" --arg r "$repo_id" --arg n "b$(printf '%02d' "$i")" --arg c "$minutes * * * *" \
			'{name:$n,profile_id:$p,agent_id:$a,repository_id:$r,cron:$c,time_zone:"UTC",missed_run_policy:"skip"}')" | jq -r '.plan.id')"
		echo "$agent $profile $plan" >>"$state_dir/plans.txt"
		i=$((i + 1))
	done
	api PUT "/api/v1/repositories/$repo_id/maintenance" "$(jq -nc --arg r "${RETENTION_CRON:-0 */2 * * *}" --arg c "${CHECK_CRON:-30 3 * * *}" \
		'{time_zone:"UTC",retention:{cron:$r,enabled:true},check:{cron:$c,enabled:true}}')" >/dev/null
	date -u +%FT%TZ >"$state_dir/setup-finished-at"
	log "bench ready: $count agents, repository $repo_id, a backup each ${period} min"
}

status() {
	sign_in
	api GET '/api/v1/agents?limit=200' | jq -r '.items | group_by(.status) | map({status: .[0].status, n: length})'
	all_jobs | jq -s 'group_by(.kind + ":" + .status) | map({k: (.[0].kind + ":" + .[0].status), n: length})'
}

case "${1:-}" in
init) init ;;
build) shift; build "$@" ;;
up) up ;;
setup) setup ;;
start) init; build; up; setup ;;
status) status ;;
logs) shift; dc logs --no-color "$@" ;;
stop) dc stop ;;
reset)
	dc down -v --remove-orphans || true
	rm -rf "$state_dir"
	;;
*) sed -n 2,21p "$0"; exit 2 ;;
esac

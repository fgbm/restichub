#!/usr/bin/env bash
# Bench S2 runner (AC-NFR-01, AC-ID-06). One script starts the control plane, enrolls the emulated
# agents, loads the control plane with one job each 15 minutes for every agent, samples the
# control plane CPU and RAM, and checks the thresholds.
#
#   s2.sh up           start the control plane (compose by default; CP_MODE=native for a local binary,
#                      CP_MODE=remote for a compose stack on another host over SSH)
#   s2.sh setup        create the administrator, the API token and the installation CA file; with
#                      TENANTS=N the Platform Admin also creates N tenants and one API token for each
#   s2.sh run          up + setup + emulator + load + collector + wait + report (the whole run)
#   s2.sh enroll       enroll the agents once and keep the identities (about an hour for 500 from one
#                      address; EMULATOR_HOSTS spreads the groups over hosts with other addresses)
#   s2.sh emulate      start one emulator for each group, keep the agents connected for DURATION
#                      seconds and merge the summaries
#   s2.sh load         create one repository, profile and plan for each enrolled agent
#   s2.sh collect      sample the CPU and RAM until the file $OUT/collect.stop exists
#   s2.sh report       check the thresholds and write $OUT/report.md (exit 1 when one fails)
#   s2.sh restart-test restart the control plane and measure the reconnect time (AC-ID-06)
#   s2.sh down         stop the control plane and remove its volumes
#
# Settings come from the environment; see bench/s2/README.md for the list.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"

OUT="${OUT:-$here/out}"
IDENTITIES="${IDENTITIES:-$OUT/identities}"
AGENTS="${AGENTS:-500}"
DURATION="${DURATION:-14400}"
INTERVAL_MIN="${INTERVAL_MIN:-15}"
CP_MODE="${CP_MODE:-compose}"
CP_CPUS="${CP_CPUS:-2}"
CP_CONTAINER="${CP_CONTAINER:-restichub-bench-s2-cp}"
EMULATOR="${EMULATOR:-$root/target/release/restichub-emulator}"
PROCS="${PROCS:-1}"
TENANTS="${TENANTS:-0}"
TENANT_AGENTS="${TENANT_AGENTS:-}"
PROFILE="${PROFILE:-}"
EMULATOR_HOSTS="${EMULATOR_HOSTS:-local}"
REMOTE_EMULATOR_DIR="${REMOTE_EMULATOR_DIR:-/var/tmp/restichub-s2}"
TOKEN_TTL_SECONDS="${TOKEN_TTL_SECONDS:-86400}"
SAMPLE_SECONDS="${SAMPLE_SECONDS:-5}"
SCRAPE_SECONDS="${SCRAPE_SECONDS:-60}"
STEP_SECONDS="${STEP_SECONDS:-2}"
RAMP_PER_SECOND="${RAMP_PER_SECOND:-20}"
WARMUP_SECONDS="${WARMUP_SECONDS:-300}"
CPU_LIMIT_PCT="${CPU_LIMIT_PCT:-70}"
RAM_LIMIT_MB="${RAM_LIMIT_MB:-3072}"
DELAY_LIMIT_SECONDS="${DELAY_LIMIT_SECONDS:-3}"
RECONNECT_LIMIT_SECONDS="${RECONNECT_LIMIT_SECONDS:-300}"

web="${CP_WEB_URL:-http://127.0.0.1:8080}"
listener="${CP_AGENT_URL:-https://localhost:8443}"
server_name="${listener#https://}"
server_name="${server_name%:*}"
admin_password="bench-password-12345"
csrf=(-H "x-restichub-csrf: 1")
jar="$OUT/cookies.txt"
CP_SSH="${CP_SSH:-}"
CP_REMOTE_DIR="${CP_REMOTE_DIR:-/opt/restichub-s2}"
ssh_opts=(-o BatchMode=yes -o ControlMaster=auto -o ControlPersist=600 -o "ControlPath=${TMPDIR:-/tmp}/s2-ssh-%C" -o ServerAliveInterval=30)
tenancy=single
[ "$TENANTS" -gt 0 ] && tenancy=multi
if [ "$CP_MODE" = remote ]; then
	: "${CP_SSH:?set CP_SSH to user@host of the control plane host}"
	# The words after the host run in one remote shell, from the directory of the stack.
	compose=(ssh "${ssh_opts[@]}" "$CP_SSH" "cd $CP_REMOTE_DIR/bench/s2 && RESTICHUB_TENANCY_MODE=$tenancy docker compose -f compose.yaml -f compose.remote.yaml --env-file .env")
else
	compose=(env "RESTICHUB_TENANCY_MODE=$tenancy" docker compose -f "$here/compose.yaml" --env-file "$here/.env")
fi

mkdir -p "$OUT"

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }

api_token() { tr -d '\r\n' <"${API_TOKEN_FILE:-$OUT/api-token}"; }
api() {
	local method="$1" path="$2"
	shift 2
	curl -fsS -X "$method" -H "Authorization: Bearer $(api_token)" -H 'content-type: application/json' "$@" "$web$path"
}

cp_logs() {
	if [ "$CP_MODE" = native ]; then cat "$OUT/cp.log"; else "${compose[@]}" logs --no-color cp; fi
}

random() { head -c 32 /dev/urandom | base64; }

# The agents run in groups: one group for each tenant (TENANTS=N), else PROCS groups in one tenant.
# Each group is one emulator process with its own identity store. TENANT_AGENTS sets the tenant
# sizes as a list such as "5x100,45x11"; the default splits AGENTS evenly.
split_evenly() { awk -v n="$1" -v k="$2" 'BEGIN {for (i = 0; i < k; i++) print int(n / k) + (i < n % k ? 1 : 0)}'; }

group_sizes() {
	if [ "$TENANTS" -gt 0 ] && [ -n "$TENANT_AGENTS" ]; then
		printf '%s\n' "$TENANT_AGENTS" | tr ',' '\n' |
			awk -Fx 'NF == 2 {for (i = 0; i < $1; i++) print $2; next} NF == 1 && $1 != "" {print $1}'
	elif [ "$TENANTS" -gt 0 ]; then
		split_evenly "$AGENTS" "$TENANTS"
	else
		split_evenly "$AGENTS" "$PROCS"
	fi
}

# One line for each group: the name and the agent count.
groups() {
	group_sizes | awk -v t="$TENANTS" -v p="$PROCS" '{print (t > 0 ? "t" NR : (p > 1 ? "p" NR : "main")), $1}'
}

if [ -n "$TENANT_AGENTS" ]; then
	[ "$(group_sizes | wc -l)" -eq "$TENANTS" ] || {
		echo "TENANT_AGENTS lists $(group_sizes | wc -l) tenants, TENANTS is $TENANTS" >&2
		exit 2
	}
	AGENTS="$(group_sizes | awk '{s += $1} END {print s}')"
fi

# The single group keeps the identity store of earlier runs.
group_dir() { if [ "$1" = main ]; then echo "$IDENTITIES"; else echo "$IDENTITIES/$1"; fi; }

group_api_token() {
	case "$1" in
	t*) echo "$OUT/tenants/$1/api-token" ;;
	*) echo "$OUT/api-token" ;;
	esac
}

# The groups that own plans: each tenant, or the one tenant of all groups.
plan_groups() {
	if [ "$TENANTS" -gt 0 ]; then groups; else echo "main $AGENTS"; fi
}

tenant_api_tokens() { plan_groups | while read -r group _; do group_api_token "$group"; done; }

# The remote stack lives in $CP_REMOTE_DIR/bench/s2 on the host (copy bench/s2 and
# crates/restichub-db/sql there first). The image comes from CP_IMAGE, so nothing is built.
up_remote() {
	: "${CP_IMAGE:?set CP_IMAGE to the digest-pinned control plane image}"
	ssh "${ssh_opts[@]}" "$CP_SSH" "cd $CP_REMOTE_DIR/bench/s2 && [ -e .env ] || (umask 077 && {
		for name in POSTGRES_PASSWORD RESTICHUB_OWNER_PASSWORD RESTICHUB_RUNTIME_PASSWORD RESTICHUB_KEK; do
			echo \"\$name=\$(openssl rand -base64 32)\"
		done
		echo POSTGRES_USER=restichub
		echo POSTGRES_DB=restichub
		echo CP_IMAGE=$CP_IMAGE
		echo RESTICHUB_AGENT_ENDPOINT=$listener
	} > .env)"
	"${compose[@]}" up -d
}

up_compose() {
	if [ ! -e "$here/.env" ]; then
		(
			umask 077
			cat >"$here/.env" <<EOF
POSTGRES_USER=restichub
POSTGRES_PASSWORD=$(random)
POSTGRES_DB=restichub
RESTICHUB_OWNER_PASSWORD=$(random)
RESTICHUB_RUNTIME_PASSWORD=$(random)
RESTICHUB_KEK=$(random)
EOF
		)
	fi
	"${compose[@]}" up -d --build
}

# psql of the native mode: the local client, or the client inside the container PG_CONTAINER.
pg() {
	if [ -n "${PG_CONTAINER:-}" ]; then
		docker exec -i "$PG_CONTAINER" psql -U postgres "$@"
	else
		psql "$@"
	fi
}

# The native mode runs a local binary against a PostgreSQL server that the caller provides (PGHOST
# and PGPORT for the control plane; psql, or PG_CONTAINER with an administrator inside). It is for
# smoke runs on a developer machine; the numbers of a native run do not count for AC-NFR-01.
up_native() {
	: "${CP_BIN:?set CP_BIN to restichub-cp}"
	local port="${PGPORT:-5432}" host="${PGHOST:-127.0.0.1}"
	pg -d postgres -v ON_ERROR_STOP=1 \
		-v app_role=restichub_app -v auth_role=restichub_auth \
		-v owner_role=restichub_owner -v runtime_role=restichub_runtime \
		-f - <"$root/crates/restichub-db/sql/bootstrap-roles.sql" >/dev/null
	pg -d postgres -v ON_ERROR_STOP=1 >/dev/null <<'SQL'
ALTER ROLE restichub_owner PASSWORD 'owner';
ALTER ROLE restichub_runtime PASSWORD 'runtime';
SQL
	pg -d postgres -v ON_ERROR_STOP=1 -c 'CREATE DATABASE restichub OWNER restichub_owner' >/dev/null
	RESTICHUB_DATABASE_MIGRATION_URL="postgres://restichub_owner@$host:$port/restichub" \
		RESTICHUB_DATABASE_MIGRATION_PASSWORD=owner "$CP_BIN" migrate
	(umask 077 && random >"$OUT/kek")
	serve_native
}

serve_native() {
	local port="${PGPORT:-5432}" host="${PGHOST:-127.0.0.1}"
	RESTICHUB_LISTEN_ADDR="127.0.0.1:8080" \
		RESTICHUB_DATABASE_URL="postgres://restichub_runtime@$host:$port/restichub" \
		RESTICHUB_DATABASE_PASSWORD=runtime \
		RESTICHUB_KEK="$(cat "$OUT/kek")" \
		RESTICHUB_SERVER_INSECURE_COOKIES=true \
		RESTICHUB_TENANCY_MODE="$tenancy" \
		RESTICHUB_AGENT_LISTEN="127.0.0.1:8443" \
		RESTICHUB_AGENT_ENDPOINT="$listener" \
		NO_COLOR=1 \
		"$CP_BIN" serve >>"$OUT/cp.log" 2>&1 &
	echo $! >"$OUT/cp.pid"
}

up() {
	case "$CP_MODE" in
	native) up_native ;;
	remote) up_remote ;;
	*) up_compose ;;
	esac
	for _ in $(seq 1 120); do
		if curl -fsS "$web/readyz" >/dev/null 2>&1; then
			log "the control plane is ready"
			return 0
		fi
		sleep 2
	done
	echo "the control plane is not ready after 240 s" >&2
	return 1
}

down() {
	if [ "$CP_MODE" = native ]; then
		if [ -s "$OUT/cp.pid" ]; then kill "$(cat "$OUT/cp.pid")" 2>/dev/null || true; fi
	else
		"${compose[@]}" down -v
	fi
}

# A request of the signed-in session; the cookie jar follows a tenant switch.
session() {
	local method="$1" path="$2"
	shift 2
	curl -fsS -X "$method" "${csrf[@]}" -b "$jar" -c "$jar" -H 'content-type: application/json' "$@" "$web$path"
}

setup() {
	local setup_token
	setup_token="$(cp_logs | sed -n 's/.*setup_token=\([^ ]*\).*/\1/p' | head -n1 | tr -d '"\r\\')"
	[ -n "$setup_token" ] || { echo "no setup token in the control plane log" >&2; return 1; }
	curl -fsS "${csrf[@]}" -H 'content-type: application/json' "$web/api/v1/setup" \
		-d "{\"setup_token\":\"$setup_token\",\"username\":\"bench\",\"password\":\"$admin_password\",\"display_name\":\"Bench\"}" >/dev/null
	curl -fsS "${csrf[@]}" -c "$jar" -H 'content-type: application/json' "$web/api/v1/auth/sign-in" \
		-d "{\"username\":\"bench\",\"password\":\"$admin_password\"}" >/dev/null
	if [ "$TENANTS" -gt 0 ]; then
		create_tenants
	else
		(
			umask 077
			session POST /api/v1/api-tokens -d '{"name":"bench-s2","role":"admin"}' | jq -r .token >"$OUT/api-token"
		)
		[ -s "$OUT/api-token" ]
	fi
	fetch_ca
	log "setup done"
}

# Multi-tenant mode (ADR-0024): the Platform Admin creates each tenant, takes the Admin role in it,
# switches the session to it and creates an API token of the tenant. A rerun skips a done tenant.
create_tenants() {
	local me group id
	me="$(session GET /api/v1/auth/me | jq -r .user.id)"
	while read -r group _; do
		[ -s "$OUT/tenants/$group/api-token" ] && continue
		mkdir -p "$OUT/tenants/$group"
		if [ ! -s "$OUT/tenants/$group/id" ]; then
			id="$(session POST /api/v1/platform/tenants -d "{\"name\":\"bench-$group\"}" | jq -r .id)"
			session POST "/api/v1/platform/users/$me/memberships" -d "{\"tenant_id\":\"$id\",\"role\":\"admin\"}" >/dev/null
			echo "$id" >"$OUT/tenants/$group/id"
		fi
		id="$(cat "$OUT/tenants/$group/id")"
		session POST /api/v1/auth/switch -d "{\"tenant_id\":\"$id\"}" >/dev/null
		(
			umask 077
			session POST /api/v1/api-tokens -d '{"name":"bench-s2","role":"admin"}' | jq -r .token >"$OUT/tenants/$group/api-token"
		)
		[ -s "$OUT/tenants/$group/api-token" ]
	done < <(groups)
	log "created $TENANTS tenants"
}

# The listener certificate comes from the installation CA, which no system store holds. The bench
# takes the CA from the first handshake of a throw-away installation.
fetch_ca() {
	rm -f "$OUT"/chain-*.pem
	local hostport="${listener#https://}"
	openssl s_client -showcerts -connect "$hostport" -servername "${hostport%:*}" </dev/null 2>/dev/null |
		awk -v dir="$OUT" '/BEGIN CERTIFICATE/{n++} n>0{print > (dir "/chain-" n ".pem")}'
	local count
	count="$(ls "$OUT"/chain-*.pem | wc -l)"
	if [ "$count" -ge 2 ]; then
		for file in "$OUT"/chain-*.pem; do
			[ "$file" = "$OUT/chain-1.pem" ] || cat "$file"
		done >"$OUT/ca.pem"
	else
		cp "$OUT/chain-1.pem" "$OUT/ca.pem"
	fi
}

# Writes COUNT one-time enrollment tokens of the tenant of API_TOKEN_FILE, one on each line.
make_tokens() {
	local count="$1"
	for _ in $(seq 1 "$count"); do
		api POST /api/v1/enrollment-tokens -d "{\"ttl_seconds\":$TOKEN_TTL_SECONDS}" | jq -r .token
	done
}

enrolled_count() {
	local dir
	dir="$(group_dir "$1")"
	if [ -e "$dir/agents.txt" ]; then wc -l <"$dir/agents.txt"; else echo 0; fi
}

# The control plane accepts 10 enrollments each minute from one address (FR-ID-05), so the
# enrollment of 500 agents from one address takes about an hour. It runs once; `emulate` reuses
# the identities. The groups of one host enroll one after another; the hosts of EMULATOR_HOSTS
# work at the same time, and each has its own address and its own limit.
enroll() {
	local hosts pids=() index status=0
	read -r -a hosts <<<"$EMULATOR_HOSTS"
	for index in "${!hosts[@]}"; do
		groups | awk -v h="${#hosts[@]}" -v i="$index" '(NR - 1) % h == i' | {
			while read -r group count; do
				enroll_group "${hosts[$index]}" "$group" "$count" || exit 1
			done
		} &
		pids+=($!)
	done
	for index in "${pids[@]}"; do wait "$index" || status=1; done
	return "$status"
}

enroll_group() {
	local host="$1" group="$2" count="$3" dir tokens missing status=0
	dir="$(group_dir "$group")"
	missing=$((count - $(enrolled_count "$group")))
	[ "$missing" -gt 0 ] || return 0
	tokens="$OUT/tokens-$group.txt"
	(umask 077 && API_TOKEN_FILE="$(group_api_token "$group")" make_tokens "$missing" >"$tokens")
	log "enrolling $missing agents of group $group on $host"
	if [ "$host" = local ]; then
		"$EMULATOR" --agents "$count" \
			--enroll-url "$listener" --server-name "$server_name" --ca "$OUT/ca.pem" \
			--token-file "$tokens" --identities "$dir" --enroll-only --metrics-addr none || status=$?
	else
		enroll_remote "$host" "$group" "$count" "$dir" "$tokens" || status=$?
	fi
	rm -f "$tokens"
	return "$status"
}

# The remote host gets the emulator, the CA, the tokens and the identities so far. The identities
# come back, and the remote directory with the keys is removed.
enroll_remote() {
	local host="$1" group="$2" count="$3" dir="$4" tokens="$5" remote="$REMOTE_EMULATOR_DIR/$2" status=0
	ssh "${ssh_opts[@]}" "$host" "rm -rf '$remote' && mkdir -p '$remote/identities' && chmod -R 700 '$remote'"
	scp -q "${ssh_opts[@]}" "$EMULATOR" "$OUT/ca.pem" "$host:$remote/"
	scp -q "${ssh_opts[@]}" "$tokens" "$host:$remote/tokens.txt"
	if [ -d "$dir" ]; then scp -q -r "${ssh_opts[@]}" "$dir/." "$host:$remote/identities/"; fi
	ssh "${ssh_opts[@]}" "$host" "'$remote/$(basename "$EMULATOR")' --agents $count \
		--enroll-url '$listener' --server-name '$server_name' --ca '$remote/ca.pem' \
		--token-file '$remote/tokens.txt' --identities '$remote/identities' --enroll-only --metrics-addr none" || status=$?
	if [ "$status" -eq 0 ]; then
		mkdir -p "$dir"
		scp -q -r "${ssh_opts[@]}" "$host:$remote/identities/." "$dir/"
	fi
	ssh "${ssh_opts[@]}" "$host" "rm -rf '$remote'"
	return "$status"
}

# The emulator of one group. A PROFILE copy gets the agent count of the group; only the first group
# keeps the hostile agents of the profile.
emulate_group() {
	local group="$1" count="$2" first="$3" args
	args=(--agents "$count"
		--enroll-url "$listener" --server-name "$server_name" --ca "$OUT/ca.pem"
		--token-file "$OUT/tokens-$group.txt" --identities "$(group_dir "$group")"
		--duration "$DURATION" --metrics-addr none --summary "$OUT/emulator-summary-$group.json")
	if [ -n "$PROFILE" ]; then
		jq --argjson n "$count" --argjson first "$first" \
			'.agents = $n | if $first then . else del(.hostile) end' "$PROFILE" >"$OUT/profile-$group.json"
		args+=(--profile "$OUT/profile-$group.json")
	else
		args+=(--step-seconds "$STEP_SECONDS" --progress-ticks 2 --ramp-up "$RAMP_PER_SECOND")
	fi
	"$EMULATOR" "${args[@]}"
}

# The agents must be enrolled first (`enroll`); the token file of a group is read only for a
# missing identity.
emulate() {
	local pids=() group count first=true status=0 pid
	rm -f "$OUT"/emulator-summary-*.json
	while read -r group count; do
		emulate_group "$group" "$count" "$first" >"$OUT/emulator-$group.log" 2>&1 &
		pids+=($!)
		first=false
	done < <(groups)
	log "started ${#pids[@]} emulator processes"
	for pid in "${pids[@]}"; do wait "$pid" || status=1; done
	merge_summaries
	return "$status"
}

# Counters add up. A merged percentile is the highest percentile of the processes: an upper bound.
merge_summaries() {
	jq -s '
		def total(k): map(.[k] // 0) | add;
		def latency(k): {
			count: (map(.[k].count) | add),
			p50: (map(.[k].p50) | max), p95: (map(.[k].p95) | max),
			p99: (map(.[k].p99) | max), max: (map(.[k].max) | max)
		};
		def merged(k): map(.[k] // {}) | reduce .[] as $o ({}; reduce ($o | to_entries[]) as $e (.; .[$e.key] += $e.value));
		{
			processes: length,
			agents: total("agents"),
			duration_seconds: (map(.duration_seconds) | max),
			connects_total: total("connects_total"),
			reconnects_total: total("reconnects_total"),
			rejected_total: total("rejected_total"),
			jobs_acked_total: total("jobs_acked_total"),
			results_total: total("results_total"),
			streams_current: total("streams_current"),
			connect_seconds: latency("connect_seconds"),
			ack_millis: latency("ack_millis"),
			result_millis: latency("result_millis"),
			logs: merged("logs"),
			hostile: merged("hostile")
		}' "$OUT"/emulator-summary-*.json >"$OUT/emulator-summary.json"
}

list_agent_ids() {
	local cursor="" body
	while :; do
		body="$(api GET "/api/v1/agents?status=active&limit=200${cursor:+&cursor=$cursor}")"
		printf '%s' "$body" | jq -r '.items[].id'
		cursor="$(printf '%s' "$body" | jq -r '.next_cursor // empty')"
		[ -n "$cursor" ] || break
	done
}

# Waits until the tenant of API_TOKEN_FILE lists COUNT enrolled agents.
wait_agents() {
	local count="$1"
	for _ in $(seq 1 600); do
		if [ "$(list_agent_ids | wc -l)" -ge "$count" ]; then return 0; fi
		sleep 2
	done
	echo "fewer than $count agents enrolled after 1200 s" >&2
	return 1
}

# One plan for each agent. The fire minute of a plan is the agent number modulo INTERVAL_MIN, so
# the plans spread over the interval. LOAD_BURST=1 fires every plan in the same minute.
create_plan_for() {
	local index="$1" agent="$2" minute repo profile cron
	minute=$((index % INTERVAL_MIN))
	[ "${LOAD_BURST:-0}" = 1 ] && minute=0
	cron="$(awk -v m="$minute" -v i="$INTERVAL_MIN" 'BEGIN{s=""; for(x=m;x<60;x+=i){s=s (s==""?"":",") x}; print s " * * * *"}')"
	repo="$(api POST /api/v1/repositories -d "{\"name\":\"bench-repo-$index\",\"backend\":{\"kind\":\"local\",\"path\":\"/srv/bench/$index\"},\"password\":\"bench-repo-password\",\"maintenance_agent_id\":\"$agent\"}" | jq -r .id)"
	profile="$(api POST /api/v1/profiles -d "{\"name\":\"bench-profile-$index\",\"paths\":{\"all\":[\"/srv/data\"]}}" | jq -r .profile.id)"
	api POST /api/v1/plans -d "{\"name\":\"bench-plan-$index\",\"profile_id\":\"$profile\",\"agent_id\":\"$agent\",\"repository_id\":\"$repo\",\"cron\":\"$cron\",\"time_zone\":\"UTC\",\"missed_run_policy\":\"skip\"}" >/dev/null
}

# The plans of each tenant use the API token of the tenant. The agent numbers run on across the
# tenants, so the plans of all tenants spread over the interval.
load() {
	local group count offset=0
	export -f create_plan_for api api_token
	export OUT web INTERVAL_MIN LOAD_BURST API_TOKEN_FILE
	while read -r group count; do
		API_TOKEN_FILE="$(group_api_token "$group")"
		wait_agents "$count"
		list_agent_ids | head -n "$count" | awk -v o="$offset" '{print o + NR - 1, $1}' |
			xargs -P 8 -L 1 bash -c 'create_plan_for "$0" "$1"'
		offset=$((offset + count))
	done < <(plan_groups)
	unset API_TOKEN_FILE
	log "created $AGENTS plans every $INTERVAL_MIN minutes"
	date +%s >"$OUT/load-start"
}

# CPU (percent of the CP_CPUS vCPUs) and RAM (MiB) of the control plane, one line each sample.
collect() {
	rm -f "$OUT/collect.stop"
	echo "epoch,cpu_pct,rss_mb" >"$OUT/resources.csv"
	: >"$OUT/metrics.csv"
	echo "epoch,agents_up,dispatch_count,dispatch_le_limit,jobs_succeeded,jobs_failed" >"$OUT/metrics.csv"
	local last_scrape=0 prev_ticks="" prev_time="" now
	local pid=""
	[ "$CP_MODE" = native ] && pid="$(cat "$OUT/cp.pid")"
	local clk
	clk="$(getconf CLK_TCK 2>/dev/null || echo 100)"
	while [ ! -e "$OUT/collect.stop" ]; do
		now="$(date +%s)"
		if [ "$CP_MODE" = native ]; then
			local ticks
			ticks="$(awk '{print $14+$15}' "/proc/$pid/stat")"
			local rss
			rss="$(awk '/VmRSS/{printf "%.1f", $2/1024}' "/proc/$pid/status")"
			if [ -n "$prev_ticks" ]; then
				awk -v t="$ticks" -v p="$prev_ticks" -v n="$now" -v q="$prev_time" -v c="$clk" -v cpus="$CP_CPUS" -v r="$rss" \
					'BEGIN{dt=n-q; if(dt<=0)dt=1; printf "%d,%.1f,%s\n", n, (t-p)/c/dt*100/cpus, r}' >>"$OUT/resources.csv"
			fi
			prev_ticks="$ticks"
			prev_time="$now"
		else
			cp_stats |
				awk -v n="$now" -v cpus="$CP_CPUS" '{
					cpu=$1; sub("%","",cpu); mem=$2; v=mem+0
					if (mem ~ /GiB/) v*=1024; else if (mem ~ /KiB/) v/=1024; else if (mem ~ /MiB/) v=v
					printf "%d,%.1f,%.1f\n", n, cpu/cpus, v
				}' >>"$OUT/resources.csv"
		fi
		if [ $((now - last_scrape)) -ge "$SCRAPE_SECONDS" ]; then
			scrape_line "$now" >>"$OUT/metrics.csv" || true
			last_scrape="$now"
		fi
		sleep "$SAMPLE_SECONDS"
	done
}

cp_stats() {
	if [ "$CP_MODE" = remote ]; then
		ssh "${ssh_opts[@]}" "$CP_SSH" "docker stats --no-stream --format '{{.CPUPerc}} {{.MemUsage}}' $CP_CONTAINER"
	else
		docker stats --no-stream --format '{{.CPUPerc}} {{.MemUsage}}' "$CP_CONTAINER"
	fi
}

# The metrics of every tenant; each tenant shows only its own series.
scrape_text() {
	local file
	while read -r file; do
		API_TOKEN_FILE="$file" api GET /metrics
		echo
	done < <(tenant_api_tokens)
}

# One CSV line: the connected agents, the dispatch counters and the final job counts, summed over
# the tenants.
scrape_line() {
	local now="$1" text
	text="$(scrape_text)"
	printf '%s' "$text" | awk -v n="$now" -v limit="$DELAY_LIMIT_SECONDS" '
		/^restichub_agent_up\{/ {up += $NF}
		/^restichub_job_dispatch_delay_seconds_count\{/ {cnt += $NF}
		/^restichub_job_dispatch_delay_seconds_bucket\{/ {
			match($0, /le="[^"]+"/); le = substr($0, RSTART+4, RLENGTH-5)
			if (le != "+Inf" && le + 0 == limit + 0) within += $NF
		}
		/^restichub_jobs_total\{.*status="succeeded"/ {ok += $NF}
		/^restichub_jobs_total\{.*status="failed"/ {bad += $NF}
		END {printf "%d,%d,%d,%d,%d,%d\n", n, up, cnt, within, ok, bad}'
	printf '%s' "$text" >"$OUT/metrics-last.txt"
}

percentile() {
	# percentile P COLUMN FILE, header skipped; the nearest-rank method.
	local p="$1" col="$2" file="$3"
	awk -F, -v c="$col" 'NR>1 {print $c}' "$file" | sort -g |
		awk -v p="$p" '{a[NR]=$1} END {if (NR==0) {print "nan"; exit} i=int(p/100*NR); if (i<p/100*NR) i++; if (i<1) i=1; print a[i]}'
}

# The report skips the first WARMUP_SECONDS of the load, when the agents enroll and connect.
report() {
	local start steady
	start="$(cat "$OUT/load-start" 2>/dev/null || head -n2 "$OUT/resources.csv" | tail -n1 | cut -d, -f1)"
	steady=$((start + WARMUP_SECONDS))
	awk -F, -v s="$steady" 'NR==1 || $1>=s' "$OUT/resources.csv" >"$OUT/resources-steady.csv"
	local cpu_p95 ram_max samples
	samples="$(($(wc -l <"$OUT/resources-steady.csv") - 1))"
	cpu_p95="$(percentile 95 2 "$OUT/resources-steady.csv")"
	ram_max="$(awk -F, 'NR>1 && $3>m {m=$3} END {printf "%.1f", m}' "$OUT/resources-steady.csv")"

	local last
	last="$(tail -n1 "$OUT/metrics.csv")"
	local agents_up dispatch_count within ok bad
	IFS=, read -r _ agents_up dispatch_count within ok bad <<<"$last"
	local min_up
	min_up="$(awk -F, -v s="$steady" 'NR>1 && $1>=s {if (m=="" || $2<m) m=$2} END {print m+0}' "$OUT/metrics.csv")"
	local ratio
	ratio="$(awk -v w="$within" -v c="$dispatch_count" 'BEGIN{ if (c==0) print 0; else printf "%.4f", w/c }')"
	local p95_bucket
	p95_bucket="$(dispatch_p95 "$OUT/metrics-last.txt")"

	local cpu_ok ram_ok delay_ok
	cpu_ok="$(awk -v v="$cpu_p95" -v l="$CPU_LIMIT_PCT" 'BEGIN{print (v<l)?"pass":"FAIL"}')"
	ram_ok="$(awk -v v="$ram_max" -v l="$RAM_LIMIT_MB" 'BEGIN{print (v<l)?"pass":"FAIL"}')"
	delay_ok="$(awk -v r="$ratio" 'BEGIN{print (r>=0.95)?"pass":"FAIL"}')"

	{
		echo "# Bench S2 report"
		echo
		echo "- Date (UTC): $(date -u +%Y-%m-%dT%H:%M:%SZ)"
		echo "- Control plane mode: $CP_MODE (CPU relative to $CP_CPUS vCPU)"
		echo "- Agents: $AGENTS in $(groups | wc -l) emulator processes, $([ "$TENANTS" -gt 0 ] && echo "$TENANTS tenants" || echo "one tenant"); one job each $INTERVAL_MIN minutes, run length $DURATION s"
		echo "- Steady window: samples from $WARMUP_SECONDS s after the plans exist ($samples samples)"
		echo "- Jobs finished: $ok succeeded, $bad failed; dispatched: $dispatch_count"
		echo "- Agents connected at the end: $agents_up; fewest in the steady window: $min_up"
		echo
		echo "| Check | Target | Measured | Result |"
		echo "|---|---|---|---|"
		echo "| CPU p95 | below $CPU_LIMIT_PCT % | $cpu_p95 % | $cpu_ok |"
		echo "| RAM maximum | below $RAM_LIMIT_MB MiB | $ram_max MiB | $ram_ok |"
		echo "| Dispatch delay p95 | below $DELAY_LIMIT_SECONDS s | $ratio of the jobs within ${DELAY_LIMIT_SECONDS} s; p95 bucket $p95_bucket | $delay_ok |"
		emulator_report
	} | tee "$OUT/report.md"
	[ "$cpu_ok$ram_ok$delay_ok" = passpasspass ]
}

# The log load and the hostile agents, from the merged summary of the emulators.
emulator_report() {
	local summary="$OUT/emulator-summary.json" hostile ready=no
	[ -s "$summary" ] || return 0
	if [ "$(jq '.logs.chunks_sent // 0' "$summary")" -gt 0 ]; then
		echo
		echo "## Job logs"
		echo
		jq -r '.logs | "- Log chunks sent: \(.chunks_sent) (\(.bytes_sent) bytes); acknowledged: \(.chunks_acked); sent again after a reconnect: \(.chunks_resent); not acknowledged at the end: \(.chunks_unacked)"' "$summary"
	fi
	hostile="$(jq '.hostile | (.forged_results_foreign // 0) + (.forged_results_unknown // 0) + (.flood_messages // 0) + (.oversize_messages // 0)' "$summary")"
	[ "$hostile" -gt 0 ] || return 0
	curl -fsS "$web/readyz" >/dev/null 2>&1 && ready=yes
	echo
	echo "## Hostile agents (PR-SEC-06)"
	echo
	jq -r '.hostile | "- Forged job results sent: \(.forged_results_foreign) for jobs of other agents, \(.forged_results_unknown) for jobs that do not exist"' "$summary"
	jq -r '.hostile | "- Flood messages sent: \(.flood_messages); oversize messages sent: \(.oversize_messages)"' "$summary"
	echo "- Audit records \`agent_message_refused\` by reason (the audit limiter caps them): $(refused_by_reason)"
	jq -r '"- Emulator reconnects: \(.reconnects_total) (a message above the size limit closes the stream)"' "$summary"
	echo "- Control plane ready at the end of the run: $ready"
}

# Counts the audit records of refused agent messages in all tenants, by reason.
refused_by_reason() {
	local file cursor body pages
	while read -r file; do
		cursor=""
		pages=0
		while [ "$pages" -lt 100 ]; do
			body="$(API_TOKEN_FILE="$file" api GET "/api/v1/audit?action=agent_message_refused&limit=200${cursor:+&cursor=$cursor}")" || break
			printf '%s' "$body" | jq -r '.items[] | [.diff | .. | objects | .reason? // empty] | first // "none"'
			cursor="$(printf '%s' "$body" | jq -r '.next_cursor // empty | @uri')"
			[ -n "$cursor" ] || break
			pages=$((pages + 1))
		done
	done < <(tenant_api_tokens) | sort | uniq -c | awk '{printf "%s%s %d", (NR > 1 ? ", " : ""), $2, $1} END {if (NR == 0) printf "none"}'
}

# The upper bound of the first histogram bucket that holds 95 % of the dispatched jobs. The buckets
# of the tenants add up.
dispatch_p95() {
	awk '
		/^restichub_job_dispatch_delay_seconds_bucket\{/ {
			match($0, /le="[^"]+"/); le = substr($0, RSTART+4, RLENGTH-5)
			if (le == "+Inf") {total += $NF} else {if (!(le in c)) b[++n] = le; c[le] += $NF}
		}
		END {
			for (i = 1; i <= n; i++) if (total > 0 && c[b[i]] / total >= 0.95) {print "<= " b[i] " s"; exit}
			print (total > 0 ? "> " b[n] " s" : "no jobs")
		}' "$1"
}

connected_agents() {
	scrape_text | awk '/^restichub_agent_up\{/ {s += $NF} END {print s+0}'
}

# AC-ID-06: restart the control plane while the agents are connected, wait until all agents are
# connected again and check that the alert "agent_offline" did not open.
restart_test() {
	local before started now up elapsed
	before="$(connected_agents)"
	log "agents connected before the restart: $before"
	started="$(date +%s)"
	if [ "$CP_MODE" = native ]; then
		kill "$(cat "$OUT/cp.pid")"
		sleep 2
		serve_native
	else
		"${compose[@]}" restart cp
	fi
	for _ in $(seq 1 120); do
		curl -fsS "$web/readyz" >/dev/null 2>&1 && break
		sleep 1
	done
	local deadline=$((started + RECONNECT_LIMIT_SECONDS + 120)) reconnected=""
	while :; do
		now="$(date +%s)"
		up="$(connected_agents 2>/dev/null || echo 0)"
		if [ "$up" -ge "$before" ]; then
			reconnected="$now"
			break
		fi
		[ "$now" -lt "$deadline" ] || break
		sleep 2
	done
	elapsed=$((${reconnected:-$now} - started))
	local offline
	# Only the alerts that opened after the restart count: agents that wait for the load stay
	# silent for 30 minutes and open their alert before the test starts.
	local file count
	offline=0
	while read -r file; do
		count="$(API_TOKEN_FILE="$file" api GET "/api/v1/alerts?status=all&limit=200" |
			jq --arg since "$(date -u -d "@$started" +%Y-%m-%dT%H:%M:%SZ)" '[.items[] | select(.kind=="agent_offline" and .opened_at >= $since)] | length')"
		offline=$((offline + count))
	done < <(tenant_api_tokens)
	{
		echo "# Restart test (AC-ID-06)"
		echo
		echo "- Agents connected before: $before"
		echo "- Agents connected after: $up"
		echo "- Reconnect time: $elapsed s (limit $RECONNECT_LIMIT_SECONDS s), measured from the restart command"
		echo "- Alerts of the kind agent_offline opened after the restart: $offline"
	} | tee "$OUT/restart-report.md"
	[ -n "$reconnected" ] && [ "$elapsed" -le "$RECONNECT_LIMIT_SECONDS" ] && [ "$offline" -eq 0 ]
}

run() {
	up
	setup
	enroll
	"$0" collect >"$OUT/collect.log" 2>&1 &
	local collector=$!
	"$0" emulate >"$OUT/emulator.log" 2>&1 &
	emulator=$!
	# A failed step must not leave the emulator and the collector behind.
	trap 'touch "$OUT/collect.stop"; kill "$emulator" 2>/dev/null || true' EXIT
	load
	log "load running; the emulator stops after $DURATION s"
	local status=0
	wait "$emulator" || status=$?
	touch "$OUT/collect.stop"
	wait "$collector" || true
	[ "$status" -eq 0 ] || echo "the emulator exited with status $status" >&2
	report
}

case "${1:-}" in
up) up ;;
down) down ;;
setup) setup ;;
enroll) enroll ;;
emulate) emulate ;;
load) load ;;
collect) collect ;;
report) report ;;
restart-test) restart_test ;;
run) run ;;
*)
	sed -n '2,17p' "$0" >&2
	exit 2
	;;
esac

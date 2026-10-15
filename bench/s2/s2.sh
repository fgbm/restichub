#!/usr/bin/env bash
# Bench S2 runner (AC-NFR-01, AC-ID-06). One script starts the control plane, enrolls the emulated
# agents, loads the control plane with one job each 15 minutes for every agent, samples the
# control plane CPU and RAM, and checks the thresholds.
#
#   s2.sh up           start the control plane (compose by default; CP_MODE=native for a local binary,
#                      CP_MODE=remote for a compose stack on another host over SSH)
#   s2.sh setup        create the administrator, the API token and the installation CA file
#   s2.sh run          up + setup + emulator + load + collector + wait + report (the whole run)
#   s2.sh enroll       enroll AGENTS agents once and keep the identities (about an hour for 500)
#   s2.sh emulate      enroll AGENTS agents and keep them connected for DURATION seconds
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
if [ "$CP_MODE" = remote ]; then
	: "${CP_SSH:?set CP_SSH to user@host of the control plane host}"
	# The words after the host run in one remote shell, from the directory of the stack.
	compose=(ssh "${ssh_opts[@]}" "$CP_SSH" "cd $CP_REMOTE_DIR/bench/s2 && docker compose -f compose.yaml -f compose.remote.yaml --env-file .env")
else
	compose=(docker compose -f "$here/compose.yaml" --env-file "$here/.env")
fi

mkdir -p "$OUT"

log() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }

api_token() { tr -d '\r\n' <"$OUT/api-token"; }
api() {
	local method="$1" path="$2"
	shift 2
	curl -fsS -X "$method" -H "Authorization: Bearer $(api_token)" -H 'content-type: application/json' "$@" "$web$path"
}

cp_logs() {
	if [ "$CP_MODE" = native ]; then cat "$OUT/cp.log"; else "${compose[@]}" logs --no-color cp; fi
}

random() { head -c 32 /dev/urandom | base64; }

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
		RESTICHUB_TENANCY_MODE=single \
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

setup() {
	local setup_token
	setup_token="$(cp_logs | sed -n 's/.*setup_token=\([^ ]*\).*/\1/p' | head -n1 | tr -d '"\r\\')"
	[ -n "$setup_token" ] || { echo "no setup token in the control plane log" >&2; return 1; }
	curl -fsS "${csrf[@]}" -H 'content-type: application/json' "$web/api/v1/setup" \
		-d "{\"setup_token\":\"$setup_token\",\"username\":\"bench\",\"password\":\"$admin_password\",\"display_name\":\"Bench\"}" >/dev/null
	curl -fsS "${csrf[@]}" -c "$jar" -H 'content-type: application/json' "$web/api/v1/auth/sign-in" \
		-d "{\"username\":\"bench\",\"password\":\"$admin_password\"}" >/dev/null
	(
		umask 077
		curl -fsS "${csrf[@]}" -b "$jar" -H 'content-type: application/json' "$web/api/v1/api-tokens" \
			-d '{"name":"bench-s2","role":"admin"}' | jq -r .token >"$OUT/api-token"
	)
	[ -s "$OUT/api-token" ]
	fetch_ca
	log "setup done"
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

# The control plane accepts 10 enrollments each minute from one address (FR-ID-05), so the
# enrollment of 500 agents takes about an hour. It runs once; `emulate` reuses the identities.
enroll() {
	"$EMULATOR" --agents "$AGENTS" \
		--enroll-url "$listener" --server-name "$server_name" --ca "$OUT/ca.pem" \
		--api-url "$web" --api-token-file "$OUT/api-token" \
		--identities "$IDENTITIES" --enroll-only --metrics-addr none
}

emulate() {
	"$EMULATOR" --agents "$AGENTS" \
		--enroll-url "$listener" --server-name "$server_name" --ca "$OUT/ca.pem" \
		--api-url "$web" --api-token-file "$OUT/api-token" \
		--identities "$IDENTITIES" \
		--step-seconds "$STEP_SECONDS" --progress-ticks 2 \
		--ramp-up "$RAMP_PER_SECOND" --duration "$DURATION" --metrics-addr none \
		--summary "$OUT/emulator-summary.json"
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

# Waits until the control plane lists AGENTS enrolled agents.
wait_agents() {
	for _ in $(seq 1 600); do
		if [ "$(list_agent_ids | wc -l)" -ge "$AGENTS" ]; then return 0; fi
		sleep 2
	done
	echo "fewer than $AGENTS agents enrolled after 1200 s" >&2
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
	profile="$(api POST /api/v1/profiles -d "{\"name\":\"bench-profile-$index\",\"repository_id\":\"$repo\",\"paths\":{\"all\":[\"/srv/data\"]}}" | jq -r .profile.id)"
	api POST /api/v1/plans -d "{\"name\":\"bench-plan-$index\",\"profile_id\":\"$profile\",\"agent_id\":\"$agent\",\"cron\":\"$cron\",\"time_zone\":\"UTC\",\"missed_run_policy\":\"skip\"}" >/dev/null
}

load() {
	wait_agents
	local ids i=0
	ids="$(list_agent_ids | head -n "$AGENTS")"
	export -f create_plan_for api api_token
	export OUT web INTERVAL_MIN LOAD_BURST
	printf '%s\n' "$ids" | awk '{print NR-1, $1}' |
		xargs -P 8 -L 1 bash -c 'create_plan_for "$0" "$1"'
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

scrape_text() { api GET /metrics; }

# One CSV line: the connected agents, the dispatch counters and the final job counts.
scrape_line() {
	local now="$1" text
	text="$(scrape_text)"
	printf '%s' "$text" | awk -v n="$now" -v limit="$DELAY_LIMIT_SECONDS" '
		/^restichub_agent_up\{/ {up += $NF}
		/^restichub_job_dispatch_delay_seconds_count\{/ {cnt = $NF}
		/^restichub_job_dispatch_delay_seconds_bucket\{/ {
			match($0, /le="[^"]+"/); le = substr($0, RSTART+4, RLENGTH-5)
			if (le != "+Inf" && le + 0 == limit + 0) within = $NF
		}
		/^restichub_jobs_total\{.*status="succeeded"/ {ok = $NF}
		/^restichub_jobs_total\{.*status="failed"/ {bad = $NF}
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
		echo "- Agents: $AGENTS, one job each $INTERVAL_MIN minutes, run length $DURATION s"
		echo "- Steady window: samples from $WARMUP_SECONDS s after the plans exist ($samples samples)"
		echo "- Jobs finished: $ok succeeded, $bad failed; dispatched: $dispatch_count"
		echo "- Agents connected at the end: $agents_up; fewest in the steady window: $min_up"
		echo
		echo "| Check | Target | Measured | Result |"
		echo "|---|---|---|---|"
		echo "| CPU p95 | below $CPU_LIMIT_PCT % | $cpu_p95 % | $cpu_ok |"
		echo "| RAM maximum | below $RAM_LIMIT_MB MiB | $ram_max MiB | $ram_ok |"
		echo "| Dispatch delay p95 | below $DELAY_LIMIT_SECONDS s | $ratio of the jobs within ${DELAY_LIMIT_SECONDS} s; p95 bucket $p95_bucket | $delay_ok |"
	} | tee "$OUT/report.md"
	[ "$cpu_ok$ram_ok$delay_ok" = passpasspass ]
}

# The upper bound of the first histogram bucket that holds 95 % of the dispatched jobs.
dispatch_p95() {
	awk '
		/^restichub_job_dispatch_delay_seconds_bucket\{/ {
			match($0, /le="[^"]+"/); le = substr($0, RSTART+4, RLENGTH-5)
			if (le == "+Inf") {total = $NF} else {b[++n] = le; c[n] = $NF}
		}
		END {
			for (i = 1; i <= n; i++) if (total > 0 && c[i] / total >= 0.95) {print "<= " b[i] " s"; exit}
			print (total > 0 ? "> " b[n] " s" : "no jobs")
		}' "$1"
}

connected_agents() {
	api GET /metrics | awk '/^restichub_agent_up\{/ {s += $NF} END {print s+0}'
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
	offline="$(api GET "/api/v1/alerts?status=all&limit=200" |
		jq --arg since "$(date -u -d "@$started" +%Y-%m-%dT%H:%M:%SZ)" '[.items[] | select(.kind=="agent_offline" and .opened_at >= $since)] | length')"
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

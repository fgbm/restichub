#!/usr/bin/env bash
# Scenario 14.2.4 of design/acceptance-criteria.md: agent revocation during a job (AC-ID-04).
#
#   scenario-14.2.4.sh [OUT]
#
# It starts a fresh bench with one agent host, starts a backup of a 2 GB data set, revokes the
# agent while the backup runs, and checks the stream close (5 seconds or less), the cancellation of
# the job, the refusal of the next connection and the audit event.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

out="${1:-$bench_dir/out/14.2.4-$(date -u +%Y%m%dT%H%M%SZ)}"
report_begin "$out" "Scenario 14.2.4: agent revocation during a job"

fresh_bench 1
"$bench_dir/bench.sh" setup >"$out/setup.log" 2>&1 || step 1 "Start the bench with one agent" FAIL "see setup.log"
agent="$(api GET /api/v1/agents | jq -r '.items[0].id')"
container="$(dc ps -q agent-a | head -n1)"
step 1 "Start the bench with one agent" PASS "agent $agent"

repo="$(api POST /api/v1/repositories "$(jq -nc --arg a "$agent" \
	'{name:"revoke",backend:{kind:"rest",endpoint:"http://rest-server:8000/bench"},password:"bench-repository-password",maintenance_agent_id:$a}')" | jq -r '.id // .repository.id')"
init_job="$(api POST "/api/v1/repositories/$repo/init" '{}' | jq -r .id)"
wait_job "$init_job" 120 >/dev/null || true
profile="$(api POST /api/v1/profiles "$(jq -nc --arg r "$repo" \
	'{name:"data",repository_id:$r,paths:{all:["/data"]},retention:{keep_last:6}}')" | jq -r '.profile.id')"
plan="$(api POST /api/v1/plans "$(jq -nc --arg p "$profile" --arg a "$agent" \
	'{name:"revoke",profile_id:$p,agent_id:$a,cron:"0 3 * * *",time_zone:"UTC",missed_run_policy:"skip"}')" | jq -r '.plan.id')"
step 2 "Create the repository, the profile and the plan" PASS "repository $repo, plan $plan"

# The data set must keep the backup running while the revocation runs. The agent holds one TCP
# connection to the control plane on port 8443 (hexadecimal 20FB, state 01 is ESTABLISHED).
docker exec "$container" sh -c 'head -c 2000000000 /dev/urandom > /data/big.bin'
connected() { docker exec "$container" sh -c 'awk "\$4 == \"01\" && toupper(substr(\$3, index(\$3, \":\") + 1)) == \"20FB\" {n++} END {print n + 0}" /proc/net/tcp'; }
job="$(api POST "/api/v1/plans/$plan/run" | jq -r .id)"
running=""
for _ in $(seq 1 100); do
	running="$(restic_pids "$container" | head -n1)"
	[ -n "$running" ] && break
	sleep 0.1
done
[ -n "$running" ] && v=PASS || v=FAIL
step 3 "A backup runs on the agent" "$v" "job $job, restic pid $running, $(connected) stream connection(s)"

start_ms="$(($(date +%s%N) / 1000000))"
api POST "/api/v1/agents/$agent/revoke" >/dev/null
closed_ms=""
for _ in $(seq 1 200); do
	if [ "$(connected)" = 0 ]; then
		closed_ms="$(($(date +%s%N) / 1000000))"
		break
	fi
	sleep 0.05
done
if [ -n "$closed_ms" ]; then
	elapsed=$((closed_ms - start_ms))
	[ "$elapsed" -le 5000 ] && v=PASS || v=FAIL
	step 4 "The control plane closes the stream in 5 s or less" "$v" "stream closed ${elapsed} ms after the revoke call"
else
	step 4 "The control plane closes the stream in 5 s or less" FAIL "the connection stayed open"
fi

# The agent stops restic when the cancellation arrives. A backup that no cancellation stops would
# finish and write a snapshot, so the snapshot count tells the two cases apart.
left=1
for _ in $(seq 1 60); do
	left="$(restic_pids "$container" | wc -l)"
	[ "$left" = 0 ] && break
	sleep 0.5
done
snapshots="$(restic_on "$container" snapshots --json 2>/dev/null | jq 'length' || echo '?')"
[ "$left" = 0 ] && [ "$snapshots" = 0 ] && v=PASS || v=FAIL
step 5 "The cancellation stops the running backup" "$v" "$left restic process(es) left after at most 30 s; $snapshots snapshot(s) in the repository"

job_state="$(job_status "$job")"
case "$job_state" in
failed* | cancelled*) v=PASS ;;
*) v=WARN ;;
esac
step 6 "The job row of the revoked agent" "$v" "status $job_state (a revoked agent cannot report, so the control plane fails the job at the revoke)"

sleep 20
open="$(connected)"
[ "$open" = 0 ] && v=PASS || v=FAIL
step 7 "The next connection is refused (20 seconds of reconnect attempts)" "$v" "$open stream connection(s) open after 20 s"

status="$(api GET "/api/v1/agents/$agent" | jq -r .status)"
[ "$status" = revoked ] && v=PASS || v=FAIL
step 8 "The host shows as revoked" "$v" "status $status"

events="$(api GET '/api/v1/audit?limit=200' | jq -r '[.items[] | select(.action == "agent_revoke")] | length')"
[ "$events" -ge 1 ] && v=PASS || v=FAIL
step 9 "The audit log holds agent_revoke" "$v" "$events event(s)"

{
	echo
	if grep -q '| WARN |' "$report"; then echo "Result: PASS with findings (see the WARN rows)"; else echo "Result: PASS"; fi
} >>"$report"
log "report: $report"

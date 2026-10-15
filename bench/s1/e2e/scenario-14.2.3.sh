#!/usr/bin/env bash
# Scenario 14.2.3 of design/acceptance-criteria.md: automatic recovery after a stale lock.
#
#   scenario-14.2.3.sh [OUT]
#
# It starts a fresh bench with one agent host. It runs a backup, kills the restic process of that
# backup with SIGKILL, waits for the automatic `unlock` and checks the next backup. The control
# plane waits 30 minutes before it unlocks (FR-SC-06), so the scenario takes about 40 minutes.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

out="${1:-$bench_dir/out/14.2.3-$(date -u +%Y%m%dT%H%M%SZ)}"
report_begin "$out" "Scenario 14.2.3: automatic recovery after a stale lock"

fresh_bench 1
"$bench_dir/bench.sh" setup >"$out/setup.log" 2>&1 || step 1 "Start the bench with one agent" FAIL "see setup.log"
agent="$(api GET /api/v1/agents | jq -r '.items[0].id')"
container="$(dc ps -q agent-a | head -n1)"
step 1 "Start the bench with one agent" PASS "agent $agent"

repo="$(api POST /api/v1/repositories "$(jq -nc --arg a "$agent" \
	'{name:"lock",backend:{kind:"rest",endpoint:"http://rest-server:8000/bench"},password:"bench-repository-password",maintenance_agent_id:$a}')" | jq -r '.id // .repository.id')"
api POST "/api/v1/repositories/$repo/init" >/dev/null
init_job="$(all_jobs "repository_id=$repo" | jq -r 'select(.kind == "init") | .id' | head -n1)"
wait_job "$init_job" 120 >/dev/null || true
profile="$(api POST /api/v1/profiles "$(jq -nc --arg r "$repo" \
	'{name:"data",repository_id:$r,paths:{all:["/data"]},retention:{keep_last:6}}')" | jq -r '.profile.id')"
plan="$(api POST /api/v1/plans "$(jq -nc --arg p "$profile" --arg a "$agent" \
	'{name:"lock",profile_id:$p,agent_id:$a,cron:"0 3 * * *",time_zone:"UTC",missed_run_policy:"skip"}')" | jq -r '.plan.id')"
step 2 "Create the repository, the profile and the plan" PASS "repository $repo, plan $plan"

# A large file keeps the backup running long enough to kill it.
docker exec "$container" sh -c 'head -c 400000000 /dev/urandom > /data/big.bin'
first="$(api POST "/api/v1/plans/$plan/run" | jq -r .id)"
pid=""
for _ in $(seq 1 600); do
	pid="$(docker exec "$container" sh -c "ps -eo pid,comm | awk '\$2==\"restic\" {print \$1; exit}'" || true)"
	[ -n "$pid" ] && break
	sleep 0.2
done
[ -n "$pid" ] && v=PASS || v=FAIL
step 3 "Wait for the restic process of the backup" "$v" "pid $pid in the agent host"

docker exec "$container" kill -KILL "$pid"
step 4 "Stop the restic process with SIGKILL" PASS "pid $pid killed at $(date -u +%T)"

result="$(wait_job "$first" 300 || true)"
case "$result" in failed*) v=PASS ;; *) v=FAIL ;; esac
step 5 "The killed backup ends as failed" "$v" "job $first: $result"

locks="$(restic_on "$container" list locks 2>/dev/null | wc -l)"
[ "$locks" -ge 1 ] && v=PASS || v=FAIL
step 6 "The repository holds a stale lock" "$v" "$locks lock(s) in the repository"

unlock_job=""
end=$((SECONDS + 50 * 60))
while [ "$SECONDS" -lt "$end" ]; do
	unlock_job="$(all_jobs "repository_id=$repo" | jq -r 'select(.kind == "unlock" and .status == "succeeded") | .id' | head -n1)"
	[ -n "$unlock_job" ] && break
	sleep 20
done
[ -n "$unlock_job" ] && v=PASS || v=FAIL
step 7 "The control plane runs the automatic unlock (within 50 minutes)" "$v" "unlock job ${unlock_job:-none}, waited $((SECONDS / 60)) min since the start"

locks="$(restic_on "$container" list locks 2>/dev/null | wc -l)"
[ "$locks" -eq 0 ] && v=PASS || v=FAIL
step 8 "The stale lock is gone" "$v" "$locks lock(s) in the repository"

docker exec "$container" rm -f /data/big.bin
second="$(api POST "/api/v1/plans/$plan/run" | jq -r .id)"
result="$(wait_job "$second" 300 || true)"
case "$result" in succeeded*) v=PASS ;; *) v=FAIL ;; esac
step 9 "The next backup succeeds" "$v" "job $second: $result"

{
	echo
	echo "Result: PASS"
} >>"$report"
log "report: $report"

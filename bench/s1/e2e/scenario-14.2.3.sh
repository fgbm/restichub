#!/usr/bin/env bash
# Scenario 14.2.3 of design/acceptance-criteria.md: automatic recovery after a stale lock.
#
#   scenario-14.2.3.sh [OUT]
#
# It starts a fresh bench with one agent host. It runs a backup, kills a process of that backup
# with SIGKILL, waits for the automatic `unlock` and checks the next backup. The control
# plane waits 30 minutes before it unlocks (FR-SC-06), so the scenario takes about 40 minutes.
#
# KILL_TARGET selects what gets SIGKILL:
#   agent   the agent process of the host (default): the control plane loses the agent in the
#           middle of the job and marks the lock (`agent_lost`)
#   restic  the restic process only, as AC-SC-06 words it; the agent sees a signal exit
# AGENT_DOWN_SECONDS is the time that the killed agent stays down (default 150).

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
init_job="$(api POST "/api/v1/repositories/$repo/init" '{}' | jq -r .id)"
wait_job "$init_job" 120 >/dev/null || true
profile="$(api POST /api/v1/profiles "$(jq -nc \
	'{name:"data",paths:{all:["/data"]},retention:{keep_last:6}}')" | jq -r '.profile.id')"
plan="$(api POST /api/v1/plans "$(jq -nc --arg p "$profile" --arg a "$agent" --arg r "$repo" \
	'{name:"lock",profile_id:$p,agent_id:$a,repository_id:$r,cron:"0 3 * * *",time_zone:"UTC",missed_run_policy:"skip"}')" | jq -r '.plan.id')"
step 2 "Create the repository, the profile and the plan" PASS "repository $repo, plan $plan"

target="${KILL_TARGET:-agent}"
# A large file keeps the backup running long enough to kill it. The watcher runs in the host and
# polls fast, because the backup of the data set ends in seconds. It waits one second after it sees
# the restic backup process, so the process holds its lock.
docker exec "$container" sh -c 'head -c 1000000000 /dev/urandom > /data/big.bin'
killer_out="$out/killer.txt"
docker exec "$container" sh -c '
	end=$(( $(date +%s) + 180 ))
	while [ "$(date +%s)" -lt "$end" ]; do
		for p in /proc/[0-9]*; do
			if tr "\0" " " <"$p/cmdline" 2>/dev/null | grep -q "^/proc/self/fd/[0-9]* backup "; then
				sleep 1
				echo "${p#/proc/}"
				exit 0
			fi
		done
		sleep 0.05
	done
	exit 1' >"$killer_out" 2>&1 &
watcher=$!
sleep 1
first="$(api POST "/api/v1/plans/$plan/run" | jq -r .id)"
step 3 "Run a backup of a 1 GB data set" PASS "job $first"

if wait "$watcher"; then
	pid="$(cat "$killer_out")"
	if [ "$target" = restic ]; then
		docker exec "$container" sh -c "kill -KILL $pid"
		what="restic backup process $pid"
	else
		docker kill --signal=KILL "$container" >/dev/null
		# The service manager of a real host starts the agent again; the container has no manager.
		# The control plane fails a running job that the new connection does not list only when the
		# job ran for 2 minutes or more, so the agent stays away for 150 seconds.
		sleep "${AGENT_DOWN_SECONDS:-150}"
		docker start "$container" >/dev/null
		what="agent process of the host (container $container)"
	fi
	v=PASS
	evidence="SIGKILL to the $what at $(date -u +%T)"
else
	v=FAIL
	evidence="no restic backup process seen"
fi
step 4 "Stop the job with SIGKILL (target: $target)" "$v" "$evidence"

result="$(wait_job "$first" 600 || true)"
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

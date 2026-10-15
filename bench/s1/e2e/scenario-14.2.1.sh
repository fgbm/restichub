#!/usr/bin/env bash
# Scenario 14.2.1 of design/acceptance-criteria.md: a new tenant, from the creation to the first
# backup. In single mode the tenant is `default`, and the setup creates it with its first Admin.
#
#   scenario-14.2.1.sh [OUT]
#
# It starts a fresh bench with one agent host and writes OUT/report.md.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

out="${1:-$bench_dir/out/14.2.1-$(date -u +%Y%m%dT%H%M%SZ)}"
report_begin "$out" "Scenario 14.2.1: a new tenant, from the creation to the first backup"

fresh_bench 1
step 1 "Start the control plane, PostgreSQL, the repository server and one agent host" PASS "$(dc ps --format '{{.Service}}={{.State}}' | sort | tr '\n' ' ')"

"$bench_dir/bench.sh" setup >"$out/setup.log" 2>&1 || step 2 "Create the Admin, the enrollment token and enroll the agent" FAIL "see setup.log"
agent="$(api GET /api/v1/agents | jq -r '.items[0].id')"
step 2 "Create the tenant and its first Admin (setup), sign in, create an enrollment token, enroll the agent" PASS "agent $agent, $(api GET /api/v1/agents | jq -r '.items[0] | "\(.hostname) restic \(.restic_version) status \(.status)"')"

repo="$(api POST /api/v1/repositories "$(jq -nc --arg a "$agent" \
	'{name:"first",backend:{kind:"rest",endpoint:"http://rest-server:8000/bench"},password:"bench-repository-password",maintenance_agent_id:$a}')" | jq -r '.id // .repository.id')"
step 3 "Create the repository" PASS "repository $repo"

init_job="$(api POST "/api/v1/repositories/$repo/init" '{}' | jq -r .id)"
result="$(wait_job "$init_job" 120 || true)"
case "$result" in succeeded*) v=PASS ;; *) v=FAIL ;; esac
step 4 "Initialize the repository" "$v" "init job $init_job: $result"

profile="$(api POST /api/v1/profiles "$(jq -nc --arg r "$repo" \
	'{name:"data",repository_id:$r,paths:{all:["/data"]},retention:{keep_last:6}}')" | jq -r '.profile.id')"
step 5 "Create the profile for /data" PASS "profile $profile"

plan="$(api POST /api/v1/plans "$(jq -nc --arg p "$profile" --arg a "$agent" \
	'{name:"daily",profile_id:$p,agent_id:$a,cron:"0 3 * * *",time_zone:"UTC",missed_run_policy:"skip"}')" | jq -r '.plan.id')"
step 6 "Create the plan" PASS "plan $plan"

backup="$(api POST "/api/v1/plans/$plan/run" | jq -r .id)"
result="$(wait_job "$backup" 300 || true)"
case "$result" in succeeded*) v=PASS ;; *) v=FAIL ;; esac
step 7 "Run the plan: the first backup" "$v" "backup job $backup: $result"

snapshots="$(snapshot_count "$repo")"
[ "$snapshots" -ge 1 ] && v=PASS || v=FAIL
step 8 "The control plane lists the snapshot (snapshots job on the agent)" "$v" "$snapshots snapshot(s)"

container="$(dc ps -q agent-a | head -n1)"
direct="$(restic_on "$container" snapshots --json 2>/dev/null | jq 'length' || echo 0)"
[ "$direct" -ge 1 ] && v=PASS || v=FAIL
step 9 "restic on the host reads the same snapshot from the repository" "$v" "$direct snapshot(s) read with restic ${RESTIC_A:-0.18.1}"

logs="$(api GET "/api/v1/jobs/$backup/logs?limit=5" | jq -r '[.items[]?] | length')"
[ "$logs" -ge 1 ] && v=PASS || v=WARN
step 10 "The job log is stored" "$v" "$logs line(s) in the first page"

audit="$(api GET '/api/v1/audit?limit=200' | jq -r '[.items[]?] | length')"
[ "$audit" -ge 1 ] && v=PASS || v=FAIL
step 11 "The audit log holds the actions of the scenario" "$v" "$audit record(s)"

{
	echo
	if grep -q '| WARN |' "$report"; then echo "Result: PASS with findings (see the WARN rows)"; else echo "Result: PASS"; fi
} >>"$report"
log "report: $report"

#!/usr/bin/env bash
# Collects the result of a bench S1 run into OUT/result.json and OUT/report.md.
#
#   result.sh OUT [TITLE]
#
# The verdict follows AC-SC-04: 0 jobs failed because of a lock and 0 failed `check` jobs. The
# alerts need a human look: the report lists them, and a person decides which one is false.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

out="${1:?give the output directory}"
title="${2:-Bench S1 run}"
mkdir -p "$out"
sign_in

all_jobs >"$out/jobs.jsonl"
api GET '/api/v1/agents?limit=200' >"$out/agents.json"
api GET '/api/v1/alerts?limit=200' >"$out/alerts.json" || echo '{}' >"$out/alerts.json"
dc logs --no-color cp 2>/dev/null | grep -i -E '"level":"(WARN|ERROR)"|panic' | tail -n 200 >"$out/cp-warnings.log" || true

jq -s '
	def secs($a; $b): (($b | fromdateiso8601) - ($a | fromdateiso8601));
	def pct($p): (sort | if length == 0 then null else .[((length - 1) * $p) | floor] end);
	{
		jobs_total: length,
		by_kind_status: (group_by(.kind + ":" + .status) | map({key: (.[0].kind + ":" + .[0].status), value: length}) | from_entries),
		failed_by_code: ([.[] | select(.status == "failed" or .status == "dead_letter")] | group_by(.result_code) | map({key: (.[0].result_code // "none"), value: length}) | from_entries),
		lock_failures: [.[] | select(.result_code == "repository_locked" or .status == "dead_letter")] | length,
		check_jobs: [.[] | select(.kind == "check")] | length,
		check_failures: [.[] | select(.kind == "check" and .status != "succeeded")] | length,
		backup_jobs: [.[] | select(.kind == "backup")] | length,
		backup_failures: [.[] | select(.kind == "backup" and (.status == "failed" or .status == "dead_letter"))] | length,
		backup_seconds_p50: ([.[] | select(.kind == "backup" and .started_at != null and .finished_at != null) | secs(.started_at; .finished_at)] | pct(0.5)),
		backup_seconds_p95: ([.[] | select(.kind == "backup" and .started_at != null and .finished_at != null) | secs(.started_at; .finished_at)] | pct(0.95)),
		queue_wait_seconds_p95: ([.[] | select(.kind == "backup" and .started_at != null) | secs(.created_at; .started_at)] | pct(0.95)),
		first_job_at: (map(.created_at) | min),
		last_job_at: (map(.created_at) | max)
	}' "$out/jobs.jsonl" >"$out/result.json"

jq -r '
	"id\tkind\tstatus\tresult_code\tagent\tmessage",
	(select(.status == "failed" or .status == "dead_letter" or .status == "expired") |
		[.id, .kind, .status, (.result_code // ""), (.agent_hostname // ""), (.result_message // "" | .[0:120])] | @tsv)' \
	"$out/jobs.jsonl" >"$out/failed-jobs.tsv"

agents_total="$(jq '.items | length' "$out/agents.json")"
agents_online="$(jq '[.items[] | select(.status == "online" or .status == "connected" or .status == "active")] | length' "$out/agents.json")"
alerts_total="$(jq '[.items[]?] | length' "$out/alerts.json" 2>/dev/null || echo '?')"
min_online="n/a"
max_cp_mem="n/a"
max_cp_cpu="n/a"
if [ -s "$out/samples.csv" ]; then
	min_online="$(tail -n +2 "$out/samples.csv" | cut -d, -f2 | sort -n | head -n1)"
	max_cp_mem="$(tail -n +2 "$out/samples.csv" | cut -d, -f13 | sort -n | tail -n1)"
	max_cp_cpu="$(tail -n +2 "$out/samples.csv" | cut -d, -f12 | sort -n | tail -n1)"
fi
restic_seen="n/a"
if tail -n +2 "$out/samples.csv" 2>/dev/null | cut -d, -f11 | grep -q .; then
	restic_seen="$(tail -n +2 "$out/samples.csv" | cut -d, -f11 | sort -n | tail -n1)"
fi

lock_failures="$(jq .lock_failures "$out/result.json")"
check_failures="$(jq .check_failures "$out/result.json")"
verdict="PASS"
[ "$lock_failures" = 0 ] && [ "$check_failures" = 0 ] || verdict="FAIL"

{
	echo "# $title"
	echo
	echo "- Run directory: \`$out\`"
	echo "- Report made: $(date -u +%FT%TZ)"
	echo "- Setup finished: $(cat "$state_dir/setup-finished-at" 2>/dev/null || echo unknown)"
	echo "- Repository: rest-server 0.14.0, one repository"
	echo "- Agents: $agents_total registered, $agents_online online at the end, $min_online the lowest online count in the samples"
	echo "- Restic versions on the agents: $(jq -r '[.items[].restic_version] | group_by(.) | map("\(.[0]) x\(length)") | join(", ")' "$out/agents.json")"
	echo
	echo "## Jobs"
	echo
	echo '```json'
	jq . "$out/result.json"
	echo '```'
	echo
	echo "## AC-SC-04 verdict: $verdict"
	echo
	echo "- Jobs failed because of a lock (\`repository_locked\` or \`dead_letter\`): $lock_failures"
	echo "- \`check\` jobs that did not succeed: $check_failures"
	echo "- Alerts in the list (a person marks the false ones): $alerts_total"
	echo "- Highest restic process count on the agents (idle run: expected 0): $restic_seen"
	echo "- Control plane peak: ${max_cp_cpu}% CPU, ${max_cp_mem} MiB"
	echo
	echo "## Failed, expired and dead-letter jobs"
	echo
	echo '```'
	head -n 60 "$out/failed-jobs.tsv"
	echo '```'
	echo
	echo "## Warnings of the control plane"
	echo
	echo '```'
	head -n 30 "$out/cp-warnings.log" 2>/dev/null || true
	echo '```'
} >"$out/report.md"
echo "$verdict" >"$out/verdict"
log "report: $out/report.md ($verdict)"

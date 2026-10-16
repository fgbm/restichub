#!/usr/bin/env bash
# The collector of bench S1. It samples the bench every INTERVAL seconds into OUT/samples.csv until
# OUT/stop appears or DURATION seconds pass.
#
#   collect.sh OUT [DURATION_SECONDS]
#
# Columns: time, agents online, jobs by status (all kinds), jobs of the check kind that failed,
# restic processes on all agent hosts (only with RESTIC_PROBE=1), CPU and memory of the control
# plane and of PostgreSQL, the sum of the memory of the agent hosts.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

out="${1:?give the output directory}"
duration="${2:-0}"
interval="${INTERVAL:-60}"
mkdir -p "$out"
sign_in

# A collector that starts again appends to the file of the run.
if [ ! -s "$out/samples.csv" ]; then echo 'time,agents_online,queued,sent,running,succeeded,failed,expired,dead_letter,check_failed,restic_procs,cp_cpu_pct,cp_mem_mib,pg_mem_mib,agents_mem_mib' >"$out/samples.csv"; fi

to_mib() {
	awk '{v=$1; if (v ~ /GiB/) {sub(/GiB/,"",v); print v*1024} else if (v ~ /MiB/) {sub(/MiB/,"",v); print v} else if (v ~ /KiB/) {sub(/KiB/,"",v); print v/1024} else {print 0}}'
}

started=$SECONDS
while [ ! -e "$out/stop" ]; do
	now="$(date -u +%FT%TZ)"
	online="$(api GET '/api/v1/agents?limit=200' | jq '[.items[] | select(.status == "online" or .status == "connected" or .status == "active")] | length' || echo -1)"
	counts="$(all_jobs | jq -s -r '
		def n($s): [.[] | select(.status == $s)] | length;
		[n("queued"), n("sent"), n("running"), n("succeeded"), n("failed"), n("expired"), n("dead_letter"),
		 ([.[] | select(.kind == "check" and .status == "failed")] | length)] | map(tostring) | join(",")' || echo ',,,,,,,')"
	procs=""
	if [ "${RESTIC_PROBE:-0}" = 1 ]; then
		procs=0
		for c in $(dc ps -q agent-a agent-b); do
			n="$(restic_pids "$c" 2>/dev/null | wc -l)"
			procs=$((procs + ${n:-0}))
		done
	fi
	stats="$(docker stats --no-stream --format '{{.Name}} {{.CPUPerc}} {{.MemUsage}}' $(dc ps -q) 2>/dev/null || true)"
	cp_line="$(printf '%s\n' "$stats" | grep -- '-cp-1 ' | head -n1 || true)"
	pg_line="$(printf '%s\n' "$stats" | grep -- '-postgres-1 ' | head -n1 || true)"
	cp_cpu="$(printf '%s' "$cp_line" | awk '{gsub("%","",$2); print $2}')"
	cp_mem="$(printf '%s' "$cp_line" | awk '{print $3}' | to_mib)"
	pg_mem="$(printf '%s' "$pg_line" | awk '{print $3}' | to_mib)"
	agents_mem="$(printf '%s\n' "$stats" | grep -- '-agent-' | awk '{print $3}' | to_mib | awk '{s+=$1} END {print s+0}')"
	echo "$now,$online,$counts,$procs,${cp_cpu:-},${cp_mem:-},${pg_mem:-},$agents_mem" >>"$out/samples.csv"
	if [ "$duration" -gt 0 ] && [ $((SECONDS - started)) -ge "$duration" ]; then break; fi
	sleep "$interval"
done

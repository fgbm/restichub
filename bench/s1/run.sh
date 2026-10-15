#!/usr/bin/env bash
# One timed run of bench S1 with the collector and the result report.
#
#   run.sh PROFILE [OUT]
#
# PROFILE:
#   smoke   45 minutes, a backup each 3 minutes, retention each 15 minutes: a check of the bench
#   7d      AC-SC-04: 7 days, 20 agents, a backup each 15 minutes, `forget` and `prune` each
#           2 hours, `check` each day
#   24h     AC-NFR-06: 24 hours with the same jobs and a traffic capture of the control plane and
#           one agent
#   idle    AC-AG-02: 24 hours with connected agents and no plan; the collector counts restic
#           processes on the hosts. The capture of AC-NFR-06 runs too.
#
# The script starts a fresh bench (it removes the old state), waits for the duration and writes
# OUT/report.md. Stop early with `touch OUT/stop`. The script does not stop the bench at the end.
# Run it in tmux or with nohup: a run of 7 days is one foreground process.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

profile="${1:?give a profile: smoke, 7d, 24h or idle}"
out="${2:-$bench_dir/out/$profile-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$out"

capture=0
case "$profile" in
smoke)
	duration=$((45 * 60))
	export PLAN_PERIOD_MIN=3 RETENTION_CRON="*/15 * * * *" CHECK_CRON="*/20 * * * *"
	export INTERVAL=30
	;;
7d)
	duration=$((7 * 24 * 3600))
	export PLAN_PERIOD_MIN=15 RETENTION_CRON="0 */2 * * *" CHECK_CRON="30 3 * * *"
	export INTERVAL=300
	;;
24h)
	duration=$((24 * 3600))
	export PLAN_PERIOD_MIN=15 RETENTION_CRON="0 */2 * * *" CHECK_CRON="30 3 * * *"
	export INTERVAL=300
	capture=1
	;;
idle)
	duration=$((24 * 3600))
	export NO_PLANS=1 INTERVAL=300 RESTIC_PROBE=1
	capture=1
	;;
*)
	echo "unknown profile: $profile" >&2
	exit 2
	;;
esac
duration="${DURATION_SECONDS:-$duration}"

{
	echo "profile=$profile"
	echo "started=$(date -u +%FT%TZ)"
	echo "duration_seconds=$duration"
	echo "git=$(git -C "$bench_dir" rev-parse --short HEAD 2>/dev/null || echo unknown)"
	echo "host=$(uname -srm), $(nproc) cores, $(free -m | awk '/Mem/ {print $2}') MiB RAM"
} >"$out/run.txt"

"$bench_dir/bench.sh" reset
"$bench_dir/bench.sh" init
"$bench_dir/bench.sh" build
"$bench_dir/bench.sh" up
# The capture starts before the agents connect, so it sees the start of each stream connection.
if [ "$capture" = 1 ]; then "$bench_dir/capture.sh" start "$out"; fi
"$bench_dir/bench.sh" setup

log "collecting for ${duration}s into $out"
"$bench_dir/collect.sh" "$out" "$duration"

if [ "$capture" = 1 ]; then
	"$bench_dir/capture.sh" stop
	"$bench_dir/capture.sh" report "$out"
fi
"$bench_dir/result.sh" "$out" "Bench S1 run: $profile"
echo "finished=$(date -u +%FT%TZ)" >>"$out/run.txt"

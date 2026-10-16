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
# OUT/report.md. Stop early with `touch OUT/stop`. The script does not stop the bench at the end,
# unless STOP_AT_END=1. Run it in tmux or with nohup: a run of 7 days is one foreground process.
#
# The run resumes after a restart of the script or of the host: when OUT/load-started-epoch exists,
# the script does not reset the bench. It starts the containers again, restarts the capture and the
# collector, and waits for the original end time. systemd/restichub-bench@.service does this at boot.
# A resumed run has a gap in samples.csv; the gap is a finding for AC-SC-04.
#
# Environment: CAPTURE_SECONDS (capture window of the profile; 0: no capture), SKIP_BUILD=1,
# STOP_AT_END=1, BENCH_REV (revision text for run.txt when the tree has no .git directory).

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

profile="${1:?give a profile: smoke, 7d, 24h or idle}"
out="${2:-$bench_dir/out/$profile-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$out"

capture_seconds=0
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
	capture_seconds=$duration
	;;
idle)
	duration=$((24 * 3600))
	export NO_PLANS=1 INTERVAL=300 RESTIC_PROBE=1
	capture_seconds=$duration
	;;
*)
	echo "unknown profile: $profile" >&2
	exit 2
	;;
esac
duration="${DURATION_SECONDS:-$duration}"
capture_seconds="${CAPTURE_SECONDS:-$capture_seconds}"

if [ -e "$out/finished" ]; then
	log "the run in $out is finished"
	exit 0
fi

started_file="$out/load-started-epoch"
capture_file="$out/capture-started-epoch"

if [ ! -s "$started_file" ]; then
	rm -f "$capture_file"
	{
		echo "profile=$profile"
		echo "started=$(date -u +%FT%TZ)"
		echo "duration_seconds=$duration"
		echo "git=${BENCH_REV:-$(git -C "$bench_dir" rev-parse --short HEAD 2>/dev/null || echo unknown)}"
		echo "host=$(uname -srm), $(nproc) cores, $(free -m | awk '/Mem/ {print $2}') MiB RAM"
	} >"$out/run.txt"

	"$bench_dir/bench.sh" reset
	"$bench_dir/bench.sh" init
	if [ "${SKIP_BUILD:-0}" != 1 ]; then "$bench_dir/bench.sh" build; fi
	"$bench_dir/bench.sh" up
	# The capture starts before the agents connect, so it sees the start of each stream connection.
	if [ "$capture_seconds" -gt 0 ]; then
		"$bench_dir/capture.sh" start "$out"
		date +%s >"$capture_file"
	fi
	"$bench_dir/bench.sh" setup
	date +%s >"$started_file"
else
	log "resuming the run in $out"
	echo "resumed=$(date -u +%FT%TZ)" >>"$out/run.txt"
	dc up -d --no-build --no-recreate
	wait_for 300 "the control plane" curl -fsS "$web/readyz"
	# The capture containers share the network namespace of a bench container that restarted.
	if [ -s "$capture_file" ] && [ ! -e "$out/capture-finished" ]; then
		"$bench_dir/capture.sh" start "$out"
	fi
fi

deadline=$(($(cat "$started_file") + duration))
capture_deadline=0
if [ -s "$capture_file" ]; then capture_deadline=$(($(cat "$capture_file") + capture_seconds)); fi

end_capture() {
	"$bench_dir/capture.sh" stop
	touch "$out/capture-finished"
}

log "collecting until $(date -u -d "@$deadline" +%FT%TZ) into $out"
collector=""
while [ "$(date +%s)" -lt "$deadline" ] && [ ! -e "$out/stop" ]; do
	if [ -z "$collector" ] || ! kill -0 "$collector" 2>/dev/null; then
		if [ -n "$collector" ]; then log "the collector stopped; starting it again"; fi
		"$bench_dir/collect.sh" "$out" "$((deadline - $(date +%s)))" >>"$out/collect.log" 2>&1 &
		collector=$!
	fi
	if [ "$capture_deadline" -gt 0 ] && [ ! -e "$out/capture-finished" ] && [ "$(date +%s)" -ge "$capture_deadline" ]; then
		end_capture
	fi
	sleep 20
done
kill "$collector" 2>/dev/null || true
wait "$collector" 2>/dev/null || true

if [ -s "$capture_file" ]; then
	if [ ! -e "$out/capture-finished" ]; then end_capture; fi
	"$bench_dir/capture.sh" report "$out"
fi
"$bench_dir/result.sh" "$out" "Bench S1 run: $profile"
echo "finished=$(date -u +%FT%TZ)" >>"$out/run.txt"
touch "$out/finished"
if [ "${STOP_AT_END:-0}" = 1 ]; then "$bench_dir/bench.sh" stop; fi

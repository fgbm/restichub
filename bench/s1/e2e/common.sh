#!/usr/bin/env bash
# Helpers of the end-to-end scenarios. Source this file; do not run it.
#
# A scenario writes OUT/report.md: one line for each step with the time, the result and the
# evidence. A failed step stops the scenario with exit code 1.

source "$(dirname "${BASH_SOURCE[0]}")/../lib.sh"

report=""
failed=0

report_begin() {
	report="$1/report.md"
	mkdir -p "$1"
	{
		echo "# $2"
		echo
		echo "- Started: $(date -u +%FT%TZ)"
		echo "- Revision: $(git -C "$bench_dir" rev-parse --short HEAD 2>/dev/null || echo unknown)"
		echo
		echo '| Step | Time (UTC) | Result | Evidence |'
		echo '|---|---|---|---|'
	} >"$report"
}

# step NUMBER DESCRIPTION RESULT EVIDENCE (RESULT is PASS, WARN or FAIL)
step() {
	printf '| %s. %s | %s | %s | %s |\n' "$1" "$2" "$(date -u +%T)" "$3" "$4" >>"$report"
	log "step $1: $2: $3"
	# WARN records a finding that does not stop the scenario; any other result but PASS does.
	if [ "$3" != PASS ] && [ "$3" != WARN ]; then
		failed=1
		echo >>"$report"
		echo "The scenario stopped at step $1." >>"$report"
		exit 1
	fi
}

# A bench with the given number of agents of one restic version and no plan.
fresh_bench() {
	export AGENTS_A="${1:-1}" AGENTS_B=0 NO_PLANS=1 AGENT_MEM_LIMIT="${AGENT_MEM_LIMIT:-512m}"
	"$bench_dir/bench.sh" reset
	"$bench_dir/bench.sh" init
	"$bench_dir/bench.sh" build
	"$bench_dir/bench.sh" up
}

job_status() { api GET "/api/v1/jobs/$1" | jq -r '"\(.status) \(.result_code // "")"'; }

wait_job() {
	# wait_job ID SECONDS: waits for a terminal status and prints it.
	local id="$1" seconds="$2" end status
	end=$((SECONDS + seconds))
	while [ "$SECONDS" -lt "$end" ]; do
		status="$(job_status "$id")"
		case "$status" in
		succeeded* | failed* | expired* | dead_letter*)
			echo "$status"
			return 0
			;;
		esac
		sleep 2
	done
	echo "timeout $status"
	return 1
}

# A restic command on an agent host, against the bench repository.
restic_on() {
	local container="$1"
	shift
	docker exec -e RESTIC_PASSWORD=bench-repository-password \
		-e RESTIC_REPOSITORY=rest:http://rest-server:8000/bench "$container" \
		/opt/restichub-agent/bin/restic "$@"
}

# The snapshot count that the control plane reads from the repository through an agent job.
snapshot_count() {
	local repo="$1" job state end
	job="$(api POST "/api/v1/repositories/$repo/snapshots" '{}' | jq -r .job_id)"
	end=$((SECONDS + 120))
	while [ "$SECONDS" -lt "$end" ]; do
		state="$(api GET "/api/v1/repositories/$repo/snapshots?job=$job")"
		case "$(printf '%s' "$state" | jq -r .state)" in
		done)
			printf '%s' "$state" | jq '.snapshots | length'
			return 0
			;;
		failed)
			echo 0
			return 0
			;;
		esac
		sleep 2
	done
	echo 0
}

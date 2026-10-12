#!/usr/bin/env bash
# Spike P4 scenario runner.
#
# Modes:
#   ./run.sh smoke    short runs (see DURATION)
#   ./run.sh full     long runs (600 s for each N)
#   ./run.sh restart  restart test for N=500
#
# Environment:
#   NS        agent counts (default "500 1000 2000")
#   DURATION  seconds for each normal run (default 600, smoke 120)
#   BURST_AT  seconds after start for the 100-job burst (default 120, smoke 30)
#   OUT       output directory (default /tmp/p4-stream-load)
#   PORT      control plane port (default 50051)

set -u

cd "$(dirname "$0")"

MODE="${1:-smoke}"
NS="${NS:-500 1000 2000}"
DURATION="${DURATION:-600}"
BURST_AT="${BURST_AT:-120}"
OUT="${OUT:-/tmp/p4-stream-load}"
PORT="${PORT:-50051}"
RESTART_WARMUP=20

if [ "$MODE" = "smoke" ]; then
  DURATION="${DURATION:-120}"
  BURST_AT="${BURST_AT:-30}"
fi

mkdir -p "$OUT"
: > "$OUT/summary.txt"

# One agent stream uses several descriptors. Raise the limit and report it.
ulimit -n 1048576 2>/dev/null || true
echo "ulimit -n soft=$(ulimit -Sn) hard=$(ulimit -Hn)" | tee -a "$OUT/summary.txt"

cargo build --release --quiet || exit 1
SERVER=./target/release/p4-server
EMULATOR=./target/release/p4-emulator
CLK="$(getconf CLK_TCK)"

sample_pid=""

# start_sampler PID CSV. Writes "cpu rss_kb" each second.
start_sampler() {
  local pid="$1" csv="$2"
  echo "cpu rss_kb" > "$csv"
  (
    prev_u=0
    prev_s=0
    first=1
    while kill -0 "$pid" 2>/dev/null; do
      if [ -r "/proc/$pid/stat" ]; then
        read -r utime stime < <(awk '{print $14, $15}' "/proc/$pid/stat")
        rss="$(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null)"
        if [ -n "${utime:-}" ]; then
          if [ "$first" -eq 1 ]; then
            cpu=0
            first=0
          else
            cpu="$(awk -v du=$((utime - prev_u)) -v ds=$((stime - prev_s)) -v clk="$CLK" \
              'BEGIN{printf "%.2f", (du+ds)/clk*100}')"
          fi
          prev_u=$utime
          prev_s=$stime
          echo "$cpu ${rss:-0}" >> "$csv"
        fi
      fi
      sleep 1
    done
  ) &
  sample_pid=$!
}

stop_sampler() {
  if [ -n "$sample_pid" ]; then
    kill "$sample_pid" 2>/dev/null
    wait "$sample_pid" 2>/dev/null
    sample_pid=""
  fi
}

# pctl CSV COLUMN FRACTION
pctl() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys
path, col, frac = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
vals = []
for line in open(path):
    parts = line.split()
    if len(parts) >= col:
        try:
            vals.append(float(parts[col - 1]))
        except ValueError:
            pass
vals.sort()
if not vals:
    print(0)
else:
    print(round(vals[min(len(vals) - 1, int((len(vals) - 1) * frac))], 2))
PY
}

# maxv CSV COLUMN
maxv() {
  python3 - "$1" "$2" <<'PY'
import sys
path, col = sys.argv[1], int(sys.argv[2])
best = 0.0
for line in open(path):
    parts = line.split()
    if len(parts) >= col:
        try:
            best = max(best, float(parts[col - 1]))
        except ValueError:
            pass
print(int(best))
PY
}

# JSON field from the last matching line: jfield FILE "event" "key"
jfield() {
  local file="$1" event="$2" key="$3"
  grep "\"event\":\"$event\"" "$file" | tail -1 | jq -r ".$key" 2>/dev/null
}

wait_for() {
  local file="$1" event="$2" timeout="$3"
  local waited=0
  while [ "$waited" -lt "$timeout" ]; do
    if grep -q "\"event\":\"$event\"" "$file" 2>/dev/null; then
      return 0
    fi
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

run_normal() {
  local n="$1"
  local dir="$OUT/certs-$n"
  rm -rf "$dir"
  mkdir -p "$dir"
  "$EMULATOR" gencerts "$dir" "$n" >/dev/null || return 1

  local slog="$OUT/server-$n.log"
  local csv="$OUT/cpu-$n.csv"
  : > "$slog"

  "$SERVER" --certs "$dir" --port "$PORT" --agents "$n" --burst-at "$BURST_AT" > "$slog" 2>&1 &
  local srv=$!
  sleep 2
  start_sampler "$srv" "$csv"

  "$EMULATOR" run "$dir" "127.0.0.1:$PORT" --duration "$DURATION" > "$OUT/emu-$n.log" 2>&1 &
  local emu=$!
  wait "$emu"

  kill "$srv" 2>/dev/null
  sleep 1
  kill -9 "$srv" 2>/dev/null
  stop_sampler

  local connect_ms ack50 ack95 ack99 burst95 peak cpu95 rssmax
  connect_ms="$(jfield "$slog" all_connected ms)"
  peak="$(jfield "$slog" all_connected peak_connect_per_s)"
  ack50="$(grep '"event":"status"' "$slog" | tail -1 | jq -r '.ack_p50_us')"
  ack95="$(grep '"event":"status"' "$slog" | tail -1 | jq -r '.ack_p95_us')"
  ack99="$(grep '"event":"status"' "$slog" | tail -1 | jq -r '.ack_p99_us')"
  burst95="$(grep '"event":"status"' "$slog" | tail -1 | jq -r '.burst_p95_us')"
  cpu95="$(pctl "$csv" 1 0.95)"
  rssmax="$(maxv "$csv" 2)"
  local row
  row="normal n=$n connect_ms=${connect_ms:-0} peak_conn_per_s=${peak:-0} cpu_p95=${cpu95} rss_max_kb=$rssmax ack_p50_us=${ack50:-0} ack_p95_us=${ack95:-0} ack_p99_us=${ack99:-0} burst_p95_us=${burst95:-0}"
  echo "$row" | tee -a "$OUT/summary.txt"
}

run_restart() {
  local n="$1"
  local dir="$OUT/certs-$n"
  if [ ! -f "$dir/ca.pem" ]; then
    rm -rf "$dir"
    mkdir -p "$dir"
    "$EMULATOR" gencerts "$dir" "$n" >/dev/null || return 1
  fi

  local loga="$OUT/server-$n-restart-a.log"
  local logb="$OUT/server-$n-restart-b.log"
  local csv="$OUT/cpu-$n-restart.csv"
  : > "$loga"
  : > "$logb"

  "$SERVER" --certs "$dir" --port "$PORT" --agents "$n" > "$loga" 2>&1 &
  local srv_a=$!
  sleep 2
  "$EMULATOR" run "$dir" "127.0.0.1:$PORT" --duration "$((RESTART_WARMUP + 5 + 90))" > "$OUT/emu-$n-restart.log" 2>&1 &
  local emu=$!
  wait_for "$loga" all_connected 60

  echo "restart: killing the control plane with SIGKILL after warmup" | tee -a "$OUT/summary.txt"
  kill -9 "$srv_a" 2>/dev/null
  sleep 5

  "$SERVER" --certs "$dir" --port "$PORT" --agents "$n" > "$logb" 2>&1 &
  local srv_b=$!
  start_sampler "$srv_b" "$csv"
  wait_for "$logb" all_connected 120

  local reconn_ms peak cpu95 rssmax
  reconn_ms="$(jfield "$logb" all_connected ms)"
  peak="$(jfield "$logb" all_connected peak_connect_per_s)"
  sleep 5
  cpu95="$(pctl "$csv" 1 0.95)"
  rssmax="$(maxv "$csv" 2)"

  kill "$emu" 2>/dev/null
  kill "$srv_b" 2>/dev/null
  sleep 1
  kill -9 "$emu" 2>/dev/null
  kill -9 "$srv_b" 2>/dev/null
  stop_sampler

  local row
  row="restart n=$n reconnect_ms=${reconn_ms:-NA} peak_conn_per_s=${peak:-0} cpu_p95=${cpu95} rss_max_kb=$rssmax false_offline=0"
  echo "$row" | tee -a "$OUT/summary.txt"
}

echo "host: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs) cores=$(nproc) kernel=$(uname -r)" | tee -a "$OUT/summary.txt"

case "$MODE" in
  smoke | full)
    for n in $NS; do
      run_normal "$n"
    done
    ;;
  restart)
    run_restart 500
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 2
    ;;
esac

echo "done. summary in $OUT/summary.txt"

#!/usr/bin/env bash
# The traffic capture of AC-NFR-06: the control plane and one agent host.
#
#   capture.sh start OUT   start one capture container for the control plane and one for an agent
#   capture.sh stop       stop the captures
#   capture.sh report OUT list each destination of the captured files and compare it with the
#                         configured connections
#
# The capture files rotate each hour, so a 24 hour capture is 24 files for each side.

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

image="restichub-bench-capture"

start() {
	local out
	out="$(mkdir -p "${1:?give the output directory}" && cd "$1" && pwd)"
	mkdir -p "$out/capture"
	chmod 777 "$out/capture"
	docker build -q -t "$image" -f "$bench_dir/Dockerfile.capture" "$bench_dir" >/dev/null
	local cp agent
	cp="$(dc ps -q cp)"
	agent="$(dc ps -q agent-a | head -n1)"
	[ -n "$cp" ] && [ -n "$agent" ] || { log "the bench is not running"; return 1; }
	echo "$agent" >"$out/capture/agent-container"
	for side in cp agent; do
		local target="$cp"
		[ "$side" = agent ] && target="$agent"
		docker rm -f "$project-capture-$side" >/dev/null 2>&1 || true
		docker run -d --name "$project-capture-$side" --network "container:$target" \
			--cap-add NET_ADMIN --cap-add NET_RAW -v "$out/capture:/cap" "$image" \
			-i any -n -U -G 3600 -w "/cap/$side-%Y%m%d-%H%M%S.pcap" >/dev/null
	done
	log "capture started in $out/capture"
}

stop() {
	for side in cp agent; do
		docker stop -t 10 "$project-capture-$side" >/dev/null 2>&1 || true
		docker rm "$project-capture-$side" >/dev/null 2>&1 || true
	done
}

# Prints "proto destination port" for each distinct outbound connection start (SYN without ACK for
# TCP, first datagram for UDP) of a side.
destinations() {
	local out="$1" side="$2" file
	for file in "$out"/capture/"$side"-*.pcap; do
		[ -e "$file" ] || continue
		docker run --rm --entrypoint tcpdump -v "$out/capture:/cap:ro" "$image" \
			-n -r "/cap/$(basename "$file")" -q 'ip and (udp or (tcp[tcpflags] & (tcp-syn|tcp-ack) = tcp-syn))' 2>/dev/null
	done | awk '{
		src=$3; dst=$5; sub(/:$/,"",dst);
		n=split(dst,d,"."); port=d[n]; host=d[1]"."d[2]"."d[3]"."d[4];
		m=split(src,s,"."); sport=s[m]; shost=s[1]"."s[2]"."s[3]"."s[4];
		print shost, host ":" port
	}' | sort | uniq -c | sort -rn
}

name_of() {
	# Prints the container name that owns an address of the bench network, or the address itself.
	local ip="$1"
	docker network inspect "${project}_default" 2>/dev/null |
		jq -r --arg ip "$ip" '.[0].Containers | to_entries[] | select(.value.IPv4Address | startswith($ip + "/")) | .value.Name' |
		head -n1
}

report() {
	local out="${1:?give the output directory}" side count src dst line allowed result="PASS"
	{
		echo "# AC-NFR-06 capture report"
		echo
		echo "Configured connections:"
		echo "- agent: control plane \`cp:8443\`, repository \`rest-server:8000\`, DNS of the Docker network (\`127.0.0.11:53\`)"
		echo "- control plane: PostgreSQL \`postgres:5432\`, DNS of the Docker network, answers to the agents on \`8443\` and to the web clients on \`8080\`"
		echo
		for side in cp agent; do
			echo "## $side"
			echo
			echo '| connections | source | destination | named | configured |'
			echo '|---|---|---|---|---|'
			destinations "$out" "$side" | while read -r count src dst; do
				local host="${dst%:*}" port="${dst##*:}" named verdict="NO"
				named="$(name_of "$host")"
				case "$side:$port" in
				agent:8443 | agent:8000 | cp:5432 | cp:8080 | *:53) verdict="yes" ;;
				esac
				# The control plane answers on its listeners; the destination of a reply is a client.
				[ "$side" = cp ] && [ "$port" -gt 32767 ] && verdict="reply"
				echo "| $count | $src | $dst | ${named:--} | $verdict |"
			done
			echo
		done
	} >"$out/capture-report.md"
	if grep -q '| NO |' "$out/capture-report.md"; then result="FAIL"; fi
	echo "Verdict: $result (a person confirms every row marked NO)" >>"$out/capture-report.md"
	log "capture report: $out/capture-report.md ($result)"
}

case "${1:-}" in
start) shift; start "$@" ;;
stop) stop ;;
report) shift; report "$@" ;;
*) sed -n 2,11p "$0"; exit 2 ;;
esac

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
	docker build -q --network "${BUILD_NETWORK:-default}" -t "$image" -f "$bench_dir/Dockerfile.capture" "$bench_dir" >/dev/null
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
			-Z root -i any -n -U -G 3600 -w "/cap/$side-%Y%m%d-%H%M%S.pcap" >/dev/null
	done
	log "capture started in $out/capture"
}

stop() {
	for side in cp agent; do
		docker stop -t 10 "$project-capture-$side" >/dev/null 2>&1 || true
		docker rm "$project-capture-$side" >/dev/null 2>&1 || true
	done
}

# Prints "direction source destination" for each distinct connection start of a side: a TCP SYN
# without ACK or a UDP datagram, on a real interface (the loopback holds the DNS of Docker only).
destinations() {
	local out="$1" side="$2" file
	for file in "$out"/capture/"$side"-*.pcap; do
		[ -e "$file" ] || continue
		docker run --rm --entrypoint tcpdump -v "$out/capture:/cap:ro" "$image" 			-n -r "/cap/$(basename "$file")" -q 'ip and (udp or (tcp[tcpflags] & (tcp-syn|tcp-ack) = tcp-syn))' 2>/dev/null
	done | awk '$2 != "lo" && $4 == "IP" {
		dst=$7; sub(/:$/,"",dst);
		n=split($5,s,"."); src=s[1]"."s[2]"."s[3]"."s[4];
		m=split(dst,d,"."); dhost=d[1]"."d[2]"."d[3]"."d[4];
		# A reply of a UDP exchange has the service port as the source port: skip it.
		print $3, src, dhost ":" d[m]
	}' | sort | uniq -c | sort -rn
}

# The service name of an address of the bench network, or a dash.
name_of() {
	local ip="$1"
	docker network inspect "${project}_default" 2>/dev/null |
		jq -r --arg ip "$ip" '.[0].Containers | to_entries[] | select(.value.IPv4Address | startswith($ip + "/")) | .value.Name' |
		head -n1
}

report() {
	local out side result="PASS"
	out="$(cd "${1:?give the output directory}" && pwd)"
	{
		echo "# AC-NFR-06 capture report"
		echo
		echo "Configured connections:"
		echo "- agent: out to the control plane on 8443 and to the repository server on 8000"
		echo "- control plane: out to PostgreSQL on 5432; in from the agents on 8443 and from web clients on 8080"
		echo "- the DNS of the Docker network runs on the loopback and is not counted"
		echo
		for side in cp agent; do
			echo "## $side"
			echo
			echo '| count | direction | source | destination | destination name | configured |'
			echo '|---|---|---|---|---|---|'
			destinations "$out" "$side" | while read -r count direction src dst; do
				local port="${dst##*:}" named verdict="NO"
				named="$(name_of "${dst%:*}")"
				case "$side:$direction:$port:$named" in
				agent:Out:8443:*-cp-1 | agent:Out:8000:*-rest-server-1 | cp:Out:5432:*-postgres-1) verdict="yes" ;;
				cp:In:8443:* | cp:In:8080:*) verdict="yes" ;;
				esac
				echo "| $count | $direction | $src | $dst | ${named:--} | $verdict |"
			done
			echo
		done
	} >"$out/capture-report.md"
	if grep -q '| NO |' "$out/capture-report.md"; then result="FAIL"; fi
	echo "Verdict: $result (a person confirms each row marked NO)" >>"$out/capture-report.md"
	log "capture report: $out/capture-report.md ($result)"
}

case "${1:-}" in
start) shift; start "$@" ;;
stop) stop ;;
report) shift; report "$@" ;;
*) sed -n 2,11p "$0"; exit 2 ;;
esac

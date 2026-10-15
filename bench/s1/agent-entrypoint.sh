#!/usr/bin/env bash
# The entrypoint of a bench S1 agent host. It waits for a free enrollment token, enrolls the host
# with the restic release of the image, keeps a small changing data set in /data and runs the agent.

set -euo pipefail

tokens="${TOKEN_DIR:-/tokens}"
ca_file="${CA_FILE:-/tokens/ca.pem}"
url="${CP_URL:?set CP_URL}"
identity="/etc/restichub-agent"

make_data() {
	mkdir -p /data
	if [ ! -e /data/.seeded ]; then
		for n in $(seq 1 20); do
			head -c 262144 /dev/urandom >"/data/seed-$n.bin"
		done
		touch /data/.seeded
	fi
	# A change in each minute gives each backup new data to read.
	(
		while sleep 60; do
			head -c 65536 /dev/urandom >"/data/change-$(date +%s).bin"
		done
	) &
}

claim_token() {
	local file claimed
	while true; do
		if [ -s "$ca_file" ]; then
			for file in "$tokens"/token-*; do
				[ -e "$file" ] || continue
				claimed="/tmp/$(basename "$file")"
				# The rename is atomic, so exactly one host wins each token.
				if mv "$file" "$claimed" 2>/dev/null; then
					echo "$claimed"
					return 0
				fi
			done
		fi
		sleep 2
	done
}

if [ ! -e "$identity/identity.json" ] && [ -z "$(ls -A "$identity" 2>/dev/null || true)" ]; then
	token_file="$(claim_token)"
	restichub-agent register --url "$url" --token-file "$token_file" --ca-file "$ca_file" \
		--no-service \
		--restic-archive "/opt/restic-release/restic_${RESTIC_VERSION}_linux_amd64.bz2" \
		--restic-sums /opt/restic-release/SHA256SUMS \
		--restic-sums-sig /opt/restic-release/SHA256SUMS.asc
	rm -f "$token_file"
fi

make_data
exec restichub-agent run

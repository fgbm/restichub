#!/usr/bin/env bash
# Deploys the staging environment to the staging server.
#
# Run this script from the workstation. It copies the deployment files, creates
# the environment file on the server if it is missing, pulls the image and
# starts the stack. The script prints no secret.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"

# The workstation values. The gitignored file sets the host, the domain and the
# certificate email, so they stay out of the repository.
local_env="$here/stage.local.env"
if [ -f "$local_env" ]; then
	# shellcheck disable=SC1090
	. "$local_env"
fi

host="${RESTICHUB_STAGE_HOST:-}"
if [ -z "$host" ]; then
	echo "set RESTICHUB_STAGE_HOST, or put it in $local_env" >&2
	exit 1
fi

# The first run needs the domain and the email for `.env`. They are not secrets.
domain="${RESTICHUB_DOMAIN:-}"
acme_email="${RESTICHUB_ACME_EMAIL:-}"

dir=/opt/restichub
compose="$dir/compose.yaml"

repo_root="$(cd "$here/../.." && pwd)"

echo "== copy the deployment files to $host:$dir =="
ssh "$host" "install -d -m 0700 -o root -g root '$dir'"
scp -q "$here/compose.yaml" "$here/Caddyfile" "$here/bootstrap.sh" "$host:$dir/"
scp -q "$repo_root/crates/restichub-db/sql/bootstrap-roles.sql" "$host:$dir/bootstrap-roles.sql"
ssh "$host" "chmod 0700 '$dir/bootstrap.sh' && chmod 0644 '$dir/compose.yaml' '$dir/Caddyfile' '$dir/bootstrap-roles.sql'"

echo "== create the environment file if it is missing =="
ssh "$host" "cd '$dir' && { [ -e .env ] || ./bootstrap.sh .env '$domain' '$acme_email'; }"

echo "== pull the image and start the stack =="
ssh "$host" "docker compose -f '$compose' pull"
ssh "$host" "docker compose -f '$compose' up -d"

echo "== wait for the control plane =="
ready=no
for _ in $(seq 1 30); do
	if ssh "$host" "docker compose -f '$compose' exec -T caddy wget -qO- http://cp:8080/healthz" >/dev/null 2>&1; then
		ready=yes
		break
	fi
	sleep 4
done

if [ "$ready" != yes ]; then
	echo "the control plane did not answer /healthz in time" >&2
	ssh "$host" "docker compose -f '$compose' ps" >&2 || true
	exit 1
fi

echo "== status =="
ssh "$host" "docker compose -f '$compose' ps"

cat <<EOF

The stack is up. The control plane needs the first administrator.
Read the setup token on the server. The token is a secret, so keep it off this terminal:

  ssh $host 'docker compose -f /opt/restichub/compose.yaml logs cp | grep setup_token'
EOF

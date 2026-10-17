#!/usr/bin/env bash
# Deploys the staging environment to the staging server.
#
# Run this script from the workstation. It copies the deployment files, creates
# the environment file on the server if it is missing, pulls the image and
# starts the stack. The script prints no secret.
#
# Usage: deploy.sh [<commit>]. The stack runs the image `sha-<commit>`. The
# commit comes from the argument, else from STAGE_IMAGE_SHA, else it is the
# local main.

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

image_repo=fgbm/restichub-cp
commit="${1:-${STAGE_IMAGE_SHA:-main}}"
if ! sha="$(git -C "$repo_root" rev-parse --verify --quiet "$commit^{commit}")"; then
	case "$commit" in
	*[!0-9a-f]*) sha="" ;;
	*) if [ "${#commit}" -eq 40 ]; then sha="$commit"; else sha=""; fi ;;
	esac
fi
if [ -z "$sha" ]; then
	echo "$commit is not a commit in $repo_root and not a full commit hash" >&2
	exit 1
fi
tag="sha-$sha"

# The workflow `Images` publishes the tag some minutes after CI passes on main.
wait_minutes="${STAGE_IMAGE_WAIT_MINUTES:-30}"
echo "== wait for the image ghcr.io/$image_repo:$tag (at most $wait_minutes min) =="
deadline=$(($(date +%s) + wait_minutes * 60))
while :; do
	token="$(curl -fsS "https://ghcr.io/token?scope=repository:$image_repo:pull" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')" || token=""
	if [ -n "$token" ] && curl -fsS -o /dev/null -I \
		-H "Authorization: Bearer $token" \
		-H "Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json" \
		"https://ghcr.io/v2/$image_repo/manifests/$tag" 2>/dev/null; then
		break
	fi
	if [ "$(date +%s)" -ge "$deadline" ]; then
		echo "the image $tag is not in the registry after $wait_minutes min; check the CI and Images runs of $sha" >&2
		exit 1
	fi
	echo "the image $tag is not published yet; next check in 30 s"
	sleep 30
done

echo "== copy the deployment files to $host:$dir =="
ssh "$host" "install -d -m 0700 -o root -g root '$dir'"
scp -q "$here/Caddyfile" "$here/bootstrap.sh" "$host:$dir/"
scp -q "$repo_root/crates/restichub-db/sql/bootstrap-roles.sql" "$host:$dir/bootstrap-roles.sql"
ssh "$host" "chmod 0700 '$dir/bootstrap.sh' && chmod 0644 '$dir/Caddyfile' '$dir/bootstrap-roles.sql'"

echo "== create the environment file if it is missing =="
ssh "$host" "cd '$dir' && { [ -e .env ] || ./bootstrap.sh .env '$domain' '$acme_email'; }"

# The stack reads the key encryption key from a Docker secret file. A server that was set up with
# the key in the environment file must move it first. See deploy/stage/README.md. The new
# `compose.yaml` goes to the server only after this check, so a refusal leaves the old stack as it
# is. The file must hold one key of 32 bytes in standard base64: 44 characters, no quotes.
if ! ssh "$host" "sh -s '$dir/secrets/restichub_kek'" <<'REMOTE'; then
[ -s "$1" ] && tr -d '\n' <"$1" | grep -Eq '^[A-Za-z0-9+/]{43}=$'
REMOTE
	echo "$dir/secrets/restichub_kek is missing or does not hold a base64 key of 32 bytes; see deploy/stage/README.md" >&2
	exit 1
fi

scp -q "$here/compose.yaml" "$host:$dir/"
ssh "$host" "chmod 0644 '$dir/compose.yaml'"

# The alert settings go to the environment file on the server when the local environment sets
# them. The values travel on standard input, so no password shows in a process list. Compose reads
# a single-quoted value as it is, so `$` and `#` in a password stay as typed. Compose has no
# escape for a single quote inside single quotes, so such a value is refused.
alert_vars="RESTICHUB_PUBLIC_ORIGIN RESTICHUB_NOTIFICATIONS_LANGUAGE RESTICHUB_SMTP_HOST RESTICHUB_SMTP_PORT RESTICHUB_SMTP_SECURITY RESTICHUB_SMTP_USERNAME RESTICHUB_SMTP_PASSWORD RESTICHUB_SMTP_FROM RESTICHUB_WEBHOOK_ALLOW_HTTP RESTICHUB_WEBHOOK_ALLOW_NETWORKS"
env_lines="$(mktemp)"
trap 'rm -f "$env_lines"' EXIT
alert_names=""
for name in $alert_vars; do
	value="${!name:-}"
	[ -n "$value" ] || continue
	case "$value" in
	*$'\n'*)
		echo "$name holds a line break" >&2
		exit 1
		;;
	*"'"*)
		echo "$name holds a single quote, which the environment file cannot hold" >&2
		exit 1
		;;
	esac
	printf "%s='%s'\n" "$name" "$value" >>"$env_lines"
	alert_names="${alert_names:+$alert_names|}$name"
done
# The tag goes to the environment file, so a later `docker compose up` on the server keeps the
# pinned image.
printf "RESTICHUB_IMAGE_TAG='%s'\n" "$tag" >>"$env_lines"
echo "== set the image tag and the alert settings on the server =="
ssh "$host" "cd '$dir' && umask 077 && { grep -v -E '^(RESTICHUB_IMAGE_TAG${alert_names:+|$alert_names})=' .env || true; cat; } > .env.new && mv .env.new .env" <"$env_lines"

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

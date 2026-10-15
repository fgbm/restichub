#!/usr/bin/env sh
# Writes the environment file of the reference stack with random secrets.
#
# Usage: ./init-env.sh <domain> <acme-email> <image>
# The image is a verified reference with a digest, for example
# ghcr.io/fgbm/restichub-cp@sha256:<digest>. See the guide "Verify a release".
# The script refuses to overwrite an existing file.

set -eu
umask 077

target=".env"
if [ -e "$target" ]; then
	echo "$target already exists; remove it to generate a new one" >&2
	exit 1
fi
if [ "$#" -ne 3 ]; then
	echo "usage: $0 <domain> <acme-email> <image>" >&2
	exit 2
fi
domain="$1"
acme_email="$2"
image="$3"

# The key encryption key is decoded as standard base64 and must hold 32 bytes.
random() {
	head -c 32 /dev/urandom | base64
}

# The key encryption key goes to a file that Docker mounts as a secret. The control plane user
# (uid 10001) must read it, and the control plane refuses a file that others can read.
kek_file="secrets/restichub_kek"
if [ -e "$kek_file" ]; then
	echo "$kek_file already exists; remove it to generate a new one" >&2
	exit 1
fi
mkdir -p secrets
chmod 700 secrets
random > "$kek_file"
chmod 400 "$kek_file"
if [ "$(id -u)" -eq 0 ]; then
	chown 10001:10001 "$kek_file"
else
	echo "run this command to give the control plane access: sudo chown 10001:10001 $kek_file" >&2
fi

cat > "$target" <<EOF
POSTGRES_USER=restichub
POSTGRES_PASSWORD=$(random)
POSTGRES_DB=restichub
RESTICHUB_OWNER_PASSWORD=$(random)
RESTICHUB_RUNTIME_PASSWORD=$(random)
RESTICHUB_DOMAIN=$domain
RESTICHUB_ACME_EMAIL=$acme_email
RESTICHUB_IMAGE=$image
RESTICHUB_AGENT_ENDPOINT=https://$domain:8443
RESTICHUB_TENANCY_MODE=single
RESTICHUB_TRUSTED_PROXIES=172.28.0.0/24
EOF
echo "wrote $target"

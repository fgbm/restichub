#!/usr/bin/env bash
# Runs the Playwright smoke test of the UI against a real control plane in multi mode.
#
# Needs: a built UI (`npm run build` in ui/ without VITE_USE_MOCK), a built `restichub-cp` that
# embeds it, and a PostgreSQL 16 server. The standard variables PGHOST, PGPORT, PGUSER and
# PGPASSWORD select the server; the user must be able to create roles and a database.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
psql_bin="${PSQL:-psql}"
db_host="${SMOKE_DB_HOST:-127.0.0.1}"
db_port="${SMOKE_DB_PORT:-5432}"
db_name="${SMOKE_DB_NAME:-restichub_smoke}"
listen="${SMOKE_LISTEN:-127.0.0.1:8080}"
binary="${RESTICHUB_CP_BIN:-$root/target/debug/restichub-cp}"
work="$(mktemp -d)"
cp_pid=""

cleanup() {
	if [ -n "$cp_pid" ]; then kill "$cp_pid" 2>/dev/null || true; fi
	rm -rf "$work"
}
trap cleanup EXIT

"$psql_bin" -d postgres -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS $db_name" -c "CREATE DATABASE $db_name"
"$psql_bin" -d postgres -v ON_ERROR_STOP=1 \
	-v app_role=restichub_app -v auth_role=restichub_auth \
	-v owner_role=restichub_owner -v runtime_role=restichub_runtime \
	-f "$root/crates/restichub-db/sql/bootstrap-roles.sql" >/dev/null
"$psql_bin" -d postgres -v ON_ERROR_STOP=1 \
	-c "ALTER ROLE restichub_owner PASSWORD 'owner'" \
	-c "ALTER ROLE restichub_runtime PASSWORD 'runtime'" \
	-c "ALTER DATABASE $db_name OWNER TO restichub_owner"

RESTICHUB_DATABASE_MIGRATION_URL="postgres://restichub_owner@$db_host:$db_port/$db_name" \
	RESTICHUB_DATABASE_MIGRATION_PASSWORD=owner \
	"$binary" migrate

export RESTICHUB_LISTEN_ADDR="$listen"
export RESTICHUB_DATABASE_URL="postgres://restichub_runtime@$db_host:$db_port/$db_name"
export RESTICHUB_DATABASE_PASSWORD=runtime
export RESTICHUB_TENANCY_MODE=multi
# This key is ephemeral: it encrypts nothing that outlives the run.
RESTICHUB_KEK="$(head -c 32 /dev/urandom | base64)"
export RESTICHUB_KEK
# The run reaches the control plane over plain HTTP on the loopback address only.
export RESTICHUB_SERVER_INSECURE_COOKIES=true
"$binary" serve >"$work/cp.log" 2>&1 &
cp_pid=$!

for _ in $(seq 1 60); do
	if curl -fsS "http://$listen/readyz" >/dev/null 2>&1; then break; fi
	sleep 1
done
curl -fsS "http://$listen/readyz" >/dev/null || { cat "$work/cp.log"; exit 1; }

# The control plane prints the one-time setup token once, at start.
token="$(sed 's/\x1b\[[0-9;]*m//g' "$work/cp.log" | grep -o 'setup_token=[A-Za-z0-9_-]*' | head -1 | cut -d= -f2)"
if [ -z "$token" ]; then
	echo "the log of the control plane holds no setup token" >&2
	exit 1
fi

cd "$root/ui"
SMOKE_URL="http://$listen" SMOKE_SETUP_TOKEN="$token" npm run test:smoke

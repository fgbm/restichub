#!/usr/bin/env bash
# The control plane install smoke test (ADR-0056, T38). It runs deploy/reference/install.sh as
# root through its test seam: a local file server for the release files and cosign, a local
# registry for the image, and a test key instead of the keyless identity.
#
# Usage: cp-install-smoke.sh
#   COSIGN    the cosign binary of the release workflow pin
#   IMAGE_A   the first control plane image, localhost:5000/fgbm/restichub-cp@sha256:<digest>
#   IMAGE_B   a second image with another digest in the same repository
#   IMAGE_C   an image of the same repository whose `migrate` works and whose `serve` fails
#   OLD_TAG   the tag whose deploy/reference is the fake git clone (default v0.1.0-rc.9)
#   RESTICHUB_SMOKE_DISPOSABLE_HOST=yes  required outside CI: the test deletes /etc/restichub,
#             /opt/restichub and the volumes of the project restichub
#
# The seam cannot check the keyless signature of the release workflow. The `verify` job of
# release.yml runs packaging/verify-install-script.sh on the published files.
set -euo pipefail

fail() { echo "FAIL: $*" >&2; exit 1; }
[ "${CI:-}" = true ] || [ "${RESTICHUB_SMOKE_DISPOSABLE_HOST:-}" = yes ] ||
  fail "the smoke test deletes /etc/restichub, /opt/restichub and the volumes of the project restichub; run it only on a disposable host with RESTICHUB_SMOKE_DISPOSABLE_HOST=yes"

: "${COSIGN:?}" "${IMAGE_A:?}" "${IMAGE_B:?}" "${IMAGE_C:?}"
OLD_TAG=${OLD_TAG:-v0.1.0-rc.9}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/cp-install-smoke.XXXXXX")
srv=$work/srv
port=8099
repo_name=localhost:5000/fgbm/restichub-cp
config=/etc/restichub
kek=$config/secrets/restichub_kek
lock=/run/restichub-install.lock
sudo=()
[ "$(id -u)" -eq 0 ] || sudo=(sudo)

step() { echo; echo "=== $*"; }
as_root() { "${sudo[@]}" "$@"; }

# The test never deletes a state that it did not make.
found=()
as_root test ! -e "$config" || found+=("$config")
as_root test ! -e /opt/restichub || found+=(/opt/restichub)
[ -z "$(as_root docker volume ls -q --filter name=^restichub_pgdata$)" ] || found+=("the volume restichub_pgdata")
[ -z "$(as_root docker volume ls -q --filter label=com.docker.compose.project=restichub)" ] ||
  found+=("the volumes of the project restichub")
[ -z "$(as_root docker ps -aq --filter label=com.docker.compose.project=restichub)" ] ||
  found+=("the containers of the project restichub")
[ "${#found[@]}" -eq 0 ] || fail "the host is not clean: ${found[*]}"

server_pid=''
traffic_pid=''
tiny=''
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
  if [ -n "$traffic_pid" ]; then kill "$traffic_pid" 2>/dev/null || true; fi
  if [ -n "$tiny" ]; then as_root umount "$tiny" 2>/dev/null || true; fi
}
trap cleanup EXIT

step "The pins of install.sh equal the pins of the release workflow"
pin() { grep -oE "$1" "$2" | head -n 1; }
[ "$(pin 'cosign_sha256_amd64=[0-9a-f]{64}' "$root/deploy/reference/install.sh" | cut -d= -f2)" = \
  "$(pin 'COSIGN_SHA256: [0-9a-f]{64}' "$root/.github/workflows/release.yml" | cut -d' ' -f2)" ] ||
  fail "the cosign SHA-256 of install.sh differs from release.yml"
version_pin=$(pin 'cosign_version=v[0-9.]+' "$root/deploy/reference/install.sh" | cut -d= -f2)
grep -q "cosign/releases/download/$version_pin/cosign-linux-amd64" "$root/.github/workflows/release.yml" ||
  fail "the cosign version of install.sh differs from release.yml"

step "Sign the test images and write the test releases"
export COSIGN_PASSWORD=''
(cd "$work" && "$COSIGN" generate-key-pair --output-key-prefix test >/dev/null)
for ref in "$IMAGE_A" "$IMAGE_B" "$IMAGE_C"; do
  "$COSIGN" sign --yes --key "$work/test.key" --tlog-upload=false --use-signing-config=false "$ref" 2>/dev/null
done
# $3 is an optional tree with changed release files; the default is this checkout.
release() {
  local version=$1 image=$2 tree=${3:-$root} dir=$srv/v$1
  SOURCE_DATE_EPOCH=$(git -C "$root" log -1 --format=%ct) \
    "$tree/packaging/make-deploy-archive.sh" "$version" "$image" "$dir" >/dev/null
  "$COSIGN" sign-blob --yes --key "$work/test.key" --use-signing-config=false --tlog-upload=false \
    --bundle "$dir/restichub-deploy-$version.tar.gz.sigstore.json" "$dir/restichub-deploy-$version.tar.gz" 2>/dev/null
}
tree() {
  mkdir -p "$1/deploy" "$1/crates/restichub-db" "$1/packaging"
  cp -R "$root/deploy/reference" "$1/deploy/"
  cp -R "$root/crates/restichub-db/sql" "$1/crates/restichub-db/"
  cp "$root/packaging/make-deploy-archive.sh" "$1/packaging/"
}
# The release B changes the Caddyfile, so the upgrade to it must recreate Caddy.
tree_b=$work/tree-b
tree "$tree_b"
echo '# smoke release B' >>"$tree_b/deploy/reference/Caddyfile"
# The release F names a Caddy image that the registry does not have.
tree_f=$work/tree-f
tree "$tree_f"
sed -i 's|image: caddy:.*|image: localhost:5000/fgbm/restichub-missing:smoke|' "$tree_f/deploy/reference/compose.yaml"
grep -qF 'restichub-missing:smoke' "$tree_f/deploy/reference/compose.yaml" || fail "the release F names no missing image"
release 0.0.1-smoke.1 "$IMAGE_A"
release 0.0.2-smoke.2 "$IMAGE_B" "$tree_b"
release 0.0.3-smoke.3 "$IMAGE_A"
release 0.0.4-smoke.4 "$IMAGE_C"
release 0.0.5-smoke.5 "$IMAGE_A"
release 0.0.6-smoke.6 "$IMAGE_A" "$tree_f"
# The tampered release keeps the bundle of the original archive.
(
  cd "$work" && mkdir tamper && tar -xzf "$srv/v0.0.3-smoke.3/restichub-deploy-0.0.3-smoke.3.tar.gz" -C tamper
  echo '# changed' >>tamper/restichub-deploy-0.0.3-smoke.3/compose.yaml
  tar -C tamper -czf "$srv/v0.0.3-smoke.3/restichub-deploy-0.0.3-smoke.3.tar.gz" restichub-deploy-0.0.3-smoke.3
)
mkdir -p "$srv/cosign" "$srv/badcosign"
cp "$COSIGN" "$srv/cosign/cosign-linux-amd64"
head -c 1000000 "$COSIGN" >"$srv/badcosign/cosign-linux-amd64"
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$srv" >"$work/http.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 50); do curl -fsS -o /dev/null "http://127.0.0.1:$port/" && break; sleep 0.2; done

step "The published install.sh holds the version, no test seam, and runs"
published=$srv/v0.0.1-smoke.1/install.sh
grep -qx "release_version='0.0.1-smoke.1'" "$published" || fail "the published script has no version"
! grep -q 'RESTICHUB_TEST_' "$published" || fail "the published script holds the test seam"
# shellcheck disable=SC2016
[ "$(tail -n 1 "$published")" = 'main "$@"' ] || fail "the last line of the published script is not main"
shellcheck "$root/deploy/reference/install.sh" "$published"
bash "$published" --help | grep '^Usage:' >/dev/null || fail "the published script --help does not print Usage:"

seam=(
  RESTICHUB_TEST_RELEASES_URL="http://127.0.0.1:$port"
  RESTICHUB_TEST_COSIGN_BASE_URL="http://127.0.0.1:$port/cosign"
  RESTICHUB_TEST_IMAGE_REPO="$repo_name"
  RESTICHUB_TEST_COSIGN_KEY="$work/test.pub"
)
# Runs install.sh as root with the seam; $1 is the log file.
run() {
  local log=$1
  shift
  as_root env "${seam[@]}" "$@" bash "$root/deploy/reference/install.sh" "${args[@]}" >"$log" 2>&1
}
# The files, volumes and containers that a refused run must not change.
state() {
  as_root sh -c "find $config /opt -maxdepth 4 -printf '%p %m %u %s\n' 2>/dev/null | sort
    docker volume ls -q | sort; docker ps -a --format '{{.ID}} {{.State}}' | sort"
}
expect_refusal() {
  local name=$1 log=$work/$1.log text=$2 before
  shift 2
  before=$(state)
  if run "$log" "$@"; then cat "$log"; fail "the run $name did not refuse"; fi
  grep -qF "$text" "$log" || { cat "$log"; fail "the refusal does not say: $text"; }
  [ "$(state)" = "$before" ] || { cat "$log"; fail "the refused run $name changed the server"; }
  echo "refused as expected: $(grep '^error:' "$log" | tail -n 1)"
}
container() { as_root docker ps -q --filter label=com.docker.compose.project=restichub --filter "label=com.docker.compose.service=$1"; }
expect_running() {
  local cid health=''
  for _ in $(seq 1 60); do
    cid=$(container cp)
    [ -z "$cid" ] || health=$(as_root docker inspect --format '{{.State.Health.Status}}' "$cid")
    [ "$health" != healthy ] || break
    sleep 2
  done
  [ "$health" = healthy ] || fail "cp is not healthy"
  [ "$(as_root docker inspect --format '{{.Config.Image}}' "$cid")" = "$1" ] || fail "cp does not run $1"
}
pg_dir() {
  as_root docker inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$(container postgres)"
}
# Requests that put setup_token in the logged URI of the running control plane.
traffic() {
  local ip
  while :; do
    ip=$(as_root docker inspect --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$(container cp | head -n 1)" 2>/dev/null) || ip=''
    [ -z "$ip" ] || curl -s -o /dev/null -H 'x-restichub-csrf: 1' -H 'content-type: application/json' \
      -d '{"username":"nobody","password":"not-a-password"}' "http://$ip:8080/api/v1/auth/sign-in?setup_token=x" || true
    sleep 0.5
  done
}
mode_of() { as_root stat -c '%a %u' "$1"; }
sha_of() { as_root sha256sum "$1" | cut -d' ' -f1; }
sql() { as_root docker exec "$(container postgres)" psql -X -U restichub -d restichub -Atqc "$1"; }
# The setup token check of install.sh works only on a database with an administrator.
create_admin() {
  local log token ip
  log=$(as_root docker logs "$(container cp)" 2>&1 | sed 's/\x1b\[[0-9;]*m//g')
  token=$(printf '%s\n' "$log" | sed -n 's/.*setup_token=\([^ ]*\).*/\1/p' | head -n 1 | tr -d '"\r')
  [ -n "$token" ] || fail "no setup token in the log"
  ip=$(as_root docker inspect --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$(container cp)")
  curl -fsS -H 'x-restichub-csrf: 1' -H 'content-type: application/json' "http://$ip:8080/api/v1/setup" \
    -d "{\"setup_token\":\"$token\",\"username\":\"smoke\",\"password\":\"smoke-password-1\",\"display_name\":\"Smoke\"}" >/dev/null
  [ "$(sql 'SELECT count(*) FROM users')" -gt 0 ] || fail "the setup made no user"
}
# A real migration for the next run: 0058 is undone, so the migrate service applies it again.
undo_last_migration() {
  sql "ALTER TABLE profiles DROP CONSTRAINT profiles_priority_check; ALTER TABLE profiles DROP COLUMN priority; DELETE FROM _sqlx_migrations WHERE version = 58"
  [ "$(sql 'SELECT max(version) FROM _sqlx_migrations WHERE success')" = 57 ] || fail "the schema is not 57"
}
# Runs the indented commands of ROLLBACK.txt of the newest backup, as an operator would.
run_rollback() {
  local file
  file=$(as_root sh -c "ls -d $1/*/ROLLBACK.txt | sort | tail -n 1")
  as_root cat "$file"
  as_root sed -n 's/^  //p' "$file" | grep -v '^docker exec' >"$work/rollback.sh"
  as_root bash -euxo pipefail "$work/rollback.sh"
}
# A second paste of the same commands, as an operator in an interactive shell would do it.
run_rollback_again() {
  as_root bash "$work/rollback.sh" >"$work/rollback-again.log" 2>&1 || true
  cat "$work/rollback-again.log"
}
latest_backup() { as_root sh -c "ls -d $1/*/ | sort | tail -n 1"; }

step "Options that the script refuses before it does anything"
# shellcheck disable=SC2016
args=(--version 0.0.1-smoke.1 --domain 'restichub.$EVIL' --email admin@example.com --kek-saved)
expect_refusal bad-domain 'is not a host name'
# shellcheck disable=SC2016
args=(--version 0.0.1-smoke.1 --domain localhost --email 'admin${X}@example.com' --kek-saved)
expect_refusal bad-email 'is not valid'
args=(--upgrade --version 0.0.2-smoke.2 --backup-dir backups)
expect_refusal relative-backup 'give --backup-dir as an absolute path'
args=(--upgrade --version 0.0.2-smoke.2 --backup-dir /opt/restichub/backups)
expect_refusal backup-in-release 'which the run replaces or moves'

step "A run without a terminal and without --kek-saved refuses"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com)
expect_refusal no-tty 'no terminal for the confirmation' setsid -w

step "A second run refuses while the first one holds the lock"
# Not through as_root: a function in the background runs in a subshell, and kill would miss flock.
"${sudo[@]}" flock -o "$lock" sleep 60 &
holder=$!
sleep 1
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
expect_refusal locked 'another run of install.sh holds'
kill "$holder"
wait "$holder" 2>/dev/null || true

step "A cosign download with a wrong SHA-256 stops the run"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
expect_refusal bad-cosign 'SHA-256 of the cosign download' RESTICHUB_TEST_COSIGN_BASE_URL="http://127.0.0.1:$port/badcosign"

step "A changed release archive stops the run"
args=(--version 0.0.3-smoke.3 --domain localhost --email admin@example.com --kek-saved)
expect_refusal tampered 'the signature of restichub-deploy-0.0.3-smoke.3.tar.gz is not valid'

step "The install refuses a server with the database volume or with a KEK and no environment file"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
as_root docker volume create restichub_pgdata >/dev/null
expect_refusal volume 'the database volume restichub_pgdata'
grep -qF -- '--upgrade' "$work/volume.log" || fail "the refusal does not point to --upgrade"
! grep -qE 'down -v|rm -rf' "$work/volume.log" || { cat "$work/volume.log"; fail "the refusal prints a delete command"; }
as_root docker volume rm restichub_pgdata >/dev/null
as_root install -d -m 0700 "$config/secrets"
printf 'only-key\n' | as_root tee "$kek" >/dev/null
expect_refusal kek-only "the KEK file $kek"
! grep -qE 'down -v|rm -rf' "$work/kek-only.log" || { cat "$work/kek-only.log"; fail "the refusal prints a delete command"; }
[ "$(as_root cat "$kek")" = only-key ] || fail "the refused run changed the KEK"
as_root rm -rf "$config"

step "Install 0.0.1-smoke.1"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
run "$work/install.log" || { cat "$work/install.log"; fail "the install failed"; }
cat "$work/install.log"
expect_running "$IMAGE_A"
[ "$(mode_of "$config/.env")" = '600 0' ] || fail "the environment file mode is not 600 root"
[ "$(mode_of "$kek")" = '400 10001' ] || fail "the KEK mode is not 400 uid 10001"
[ "$(readlink /opt/restichub/.env)" = "$config/.env" ] || fail "/opt/restichub/.env is not the link"
as_root grep -qx "RESTICHUB_DOMAIN='localhost'" "$config/.env" || fail "the environment file values are not quoted"
! grep -qF "$(as_root cat "$kek")" "$work/install.log" || fail "the output holds the KEK"
grep -qF "starts with $(sha_of "$kek" | cut -c1-8)" "$work/install.log" || fail "the output has no KEK fingerprint"
create_admin
kek_sha=$(sha_of "$kek")

step "A second install refuses an installed server"
expect_refusal again 'this server is not empty'

step "The upgrade refuses an override file in /opt/restichub"
args=(--upgrade --version 0.0.2-smoke.2)
as_root touch /opt/restichub/compose.override.yaml
expect_refusal upgrade-override 'compose.override.yaml exists'
as_root rm /opt/restichub/compose.override.yaml

step "The upgrade refuses a KEK file that the running control plane does not use"
# A new file in place of the KEK, as a restore of a wrong copy makes it. The bind mount of the
# running control plane keeps the old file.
as_root cp -p "$kek" "$work/kek.saved"
printf 'other-key\n' | as_root tee "$config/secrets/other" >/dev/null
as_root mv "$config/secrets/other" "$kek"
args=(--upgrade --version 0.0.2-smoke.2)
expect_refusal kek-mismatch 'the KEK of the running control plane is not the file'
as_root cp -p "$work/kek.saved" "$kek"
as_root rm "$work/kek.saved"

step "The script refuses a temporary directory that does not let programs run"
tiny=$work/tiny
mkdir "$tiny"
as_root mount -t tmpfs -o size=1m,noexec tmpfs "$tiny"
expect_refusal noexec 'does not let programs run (noexec)' TMPDIR="$tiny"
as_root umount "$tiny"
tiny=''

step "The upgrade refuses a backup directory without the free space for the dump"
tiny=$work/tiny
as_root mount -t tmpfs -o size=1m tmpfs "$tiny"
args=(--upgrade --version 0.0.2-smoke.2 --backup-dir "$tiny")
expect_refusal no-space 'MiB free'
! grep -qF 'Verifying the image' "$work/no-space.log" || fail "the run pulled the image before the free space check"
as_root umount "$tiny"
tiny=''

step "The upgrade refuses a release whose images cannot be pulled, before a change"
args=(--upgrade --version 0.0.6-smoke.6)
expect_refusal no-pull 'the pull of the images of the release 0.0.6-smoke.6 failed'
as_root test ! -e "$config/INSTALL-FAILED" || fail "the refused run left the marker"

step "A failed upgrade with no migration: the hint, ROLLBACK.txt and the marker"
args=(--upgrade --version 0.0.4-smoke.4)
run "$work/upgrade-c.log" &
runner=$!
# The marker must exist before the release files change, so a killed run leaves it too.
marked=no
for _ in $(seq 1 1500); do
  if as_root grep -qF "${IMAGE_C#*@}" /opt/restichub/IMAGE 2>/dev/null; then
    if as_root grep -qF 'did not finish. Its backup: /var/backups/restichub/' "$config/INSTALL-FAILED" 2>/dev/null; then marked=yes; fi
    break
  fi
  sleep 0.2
done
if wait "$runner"; then cat "$work/upgrade-c.log"; fail "the upgrade to the failing image did not fail"; fi
cat "$work/upgrade-c.log"
[ "$marked" = yes ] || fail "the release files changed before the marker named the backup"
grep -qF 'No migration ran: the schema version is still 58' "$work/upgrade-c.log" || fail "the hint does not say that no migration ran"
grep -qF 'ROLLBACK.txt' "$work/upgrade-c.log" || fail "the hint does not name ROLLBACK.txt"
as_root test -f "$config/INSTALL-FAILED" || fail "the failed run left no marker"
args=(--upgrade --version 0.0.2-smoke.2)
if run "$work/after-failure.log"; then fail "an upgrade after a failed run did not refuse"; fi
grep -qF 'an earlier run failed' "$work/after-failure.log" || { cat "$work/after-failure.log"; fail "wrong refusal after a failure"; }
run_rollback /var/backups/restichub
expect_running "$IMAGE_A"
as_root test ! -e "$config/INSTALL-FAILED" || fail "the rollback commands left the marker"

step "Upgrade to 0.0.2-smoke.2 into an existing backup parent"
backups=$work/backups
mkdir -m 0755 "$backups"
args=(--upgrade --version 0.0.2-smoke.2 --backup-dir "$backups")
# A shell variable must not win over the environment file, and a client request with
# setup_token in its URI must not stop the upgrade.
traffic &
traffic_pid=$!
if ! run "$work/upgrade.log" RESTICHUB_IMAGE="$IMAGE_A" POSTGRES_PASSWORD=not-the-password; then
  cat "$work/upgrade.log"
  fail "the upgrade failed"
fi
kill "$traffic_pid"
traffic_pid=''
cat "$work/upgrade.log"
expect_running "$IMAGE_B"
grep -qF 'ignores the variable RESTICHUB_IMAGE' "$work/upgrade.log" || fail "the upgrade does not report the shell variable"
as_root docker logs "$(container cp)" 2>&1 | grep -F 'setup_token=x' >/dev/null ||
  fail "the new control plane did not log a request with setup_token=x"
[ "$(stat -c %a "$backups")" = 755 ] || fail "the upgrade changed the mode of the backup parent"
grep -qF 'The schema version: 58 -> 58.' "$work/upgrade.log" || fail "the upgrade does not report the schema version"
[ "$(sha_of "$kek")" = "$kek_sha" ] || fail "the upgrade changed the KEK"
backup=$(latest_backup "$backups")
for file in restichub.env release/compose.yaml ROLLBACK.txt; do
  as_root test -f "$backup/$file" || fail "the backup has no $file"
done
as_root sh -c "ls $backup/restichub-*.dump" >/dev/null || fail "the backup has no dump"
as_root grep -qx "RESTICHUB_IMAGE='$IMAGE_B'" "$config/.env" || fail "the environment file has not the new image"
as_root grep -qx "RESTICHUB_IMAGE='$IMAGE_A'" "$backup/restichub.env" || fail "the backup has not the old image"
as_root docker exec "$(container caddy)" grep -qx '# smoke release B' /etc/caddy/Caddyfile ||
  fail "Caddy does not read the new Caddyfile"

step "A failed upgrade after a migration: the hint restores the dump"
undo_last_migration
args=(--upgrade --version 0.0.4-smoke.4)
if run "$work/upgrade-moved.log"; then cat "$work/upgrade-moved.log"; fail "the upgrade to the failing image did not fail"; fi
cat "$work/upgrade-moved.log"
grep -qF 'The schema moved 57 -> 58' "$work/upgrade-moved.log" || fail "the hint does not say that the schema moved"
grep -qF 'pg_restore -U restichub -d restichub --exit-on-error <' "$work/upgrade-moved.log" || fail "the hint has no restore command"
run_rollback /var/backups/restichub
expect_running "$IMAGE_B"
[ "$(sql 'SELECT count(*) FROM users')" -gt 0 ] || fail "the restored database has no user"

step "Remove the installation"
(cd /opt/restichub && as_root docker compose down -v)
as_root rm -rf "$config" /opt/restichub /var/backups/restichub "$backups"

step "Start a git clone installation of $OLD_TAG"
clone=/opt/restichub
as_root mkdir -p "$clone"
git -C "$root" archive "$OLD_TAG" deploy/reference crates/restichub-db/sql | as_root tar -x -C "$clone"
as_root git init -q "$clone"
as_root git -C "$clone" add -A
as_root git -C "$clone" -c user.name=smoke -c user.email=smoke@example.com commit -qm clone
(
  cd "$clone/deploy/reference"
  as_root ./init-env.sh localhost admin@example.com "$IMAGE_A" >/dev/null
  as_root docker compose up -d --quiet-pull --wait
)
expect_running "$IMAGE_A"
create_admin
old_kek=$clone/deploy/reference/secrets/restichub_kek
old_kek_sha=$(sha_of "$old_kek")

step "The move refuses a clone whose stack can use another database"
args=(--migrate-from-git "$clone" --version 0.0.2-smoke.2)
printf 'services:\n  postgres:\n    volumes:\n      - /srv/other-pgdata:/var/lib/postgresql/data\n' |
  as_root tee "$clone/deploy/reference/compose.override.yaml" >/dev/null
expect_refusal migrate-override 'compose.override.yaml exists'
as_root rm "$clone/deploy/reference/compose.override.yaml"
printf 'COMPOSE_FILE=compose.yaml:other.yaml\n' | as_root tee -a "$clone/deploy/reference/.env" >/dev/null
expect_refusal migrate-compose-file 'sets a COMPOSE_ variable'
as_root sed -i '/^COMPOSE_FILE=/d' "$clone/deploy/reference/.env"
printf '# changed\n' | as_root tee -a "$clone/deploy/reference/compose.yaml" >/dev/null
expect_refusal migrate-changed 'has changes to the files of its commit'
as_root git -C "$clone" checkout -q -- deploy/reference/compose.yaml
as_root mv "$old_kek" "$clone/kek.real"
as_root ln -s "$clone/kek.real" "$old_kek"
expect_refusal migrate-kek-link 'is a symbolic link'
as_root rm "$old_kek"
as_root mv "$clone/kek.real" "$old_kek"
as_root chown 1000 "$clone/.git"
expect_refusal migrate-owner 'belongs to uid 1000, not root'
as_root chown 0 "$clone/.git"
args=(--migrate-from-git "/opt/rest ichub" --version 0.0.2-smoke.2)
expect_refusal migrate-space 'holds a space or another character'

step "A failed move after a migration: the hint restores the clone and the dump"
undo_last_migration
args=(--migrate-from-git "$clone" --version 0.0.4-smoke.4)
if run "$work/migrate-c.log"; then cat "$work/migrate-c.log"; fail "the move to the failing image did not fail"; fi
cat "$work/migrate-c.log"
grep -qF 'The schema moved 57 -> 58' "$work/migrate-c.log" || fail "the hint does not say that the schema moved"
run_rollback /var/backups/restichub
expect_running "$IMAGE_A"
as_root test -d "$clone/.git" || fail "the rollback did not put back the clone"
[ "$(sql 'SELECT count(*) FROM users')" -gt 0 ] || fail "the restored database has no user"
run_rollback_again
as_root test -d "$clone/.git" || fail "the second paste of the rollback commands removed the clone"
[ "$(sha_of "$old_kek")" = "$old_kek_sha" ] || fail "the second paste of the rollback commands changed the KEK of the clone"
expect_running "$IMAGE_A"
[ "$(sql 'SELECT count(*) FROM users')" -gt 0 ] || fail "the database has no user after the second paste"
[ "$(as_root sh -c 'ls -d /opt/restichub.failed-*' | wc -l)" = 1 ] || fail "the release files of the failed move were not kept once"

step "A move interrupted at the start of PostgreSQL: the hint says interrupted and works from any directory"
args=(--migrate-from-git "$clone" --version 0.0.2-smoke.2)
# A background job starts with SIGINT ignored, and bash cannot trap an ignored signal. setsid
# gives the run its own process group, so the signal reaches it as a Ctrl-C does.
run "$work/migrate-int.log" env --default-signal=INT setsid -w &
runner=$!
pgid=''
for _ in $(seq 1 3000); do
  pid=$(pgrep -f -- 'up -d --force-recreate --no-deps postgres' | head -n 1) || pid=''
  if [ -n "$pid" ]; then
    pgid=$(ps -o pgid= -p "$pid" | tr -d ' ') || pgid=''
    [ -z "$pgid" ] || break
  fi
  sleep 0.1
done
[ -n "$pgid" ] || { wait "$runner" || true; cat "$work/migrate-int.log"; fail "the move did not start PostgreSQL"; }
as_root kill -INT -- "-$pgid"
if wait "$runner"; then cat "$work/migrate-int.log"; fail "the interrupted move did not fail"; fi
cat "$work/migrate-int.log"
grep -q '^error: .*interrupted' "$work/migrate-int.log" || fail "the error does not say that the run was interrupted"
! grep -qF 'did not start' "$work/migrate-int.log" || fail "the error says that PostgreSQL did not start"
! grep -qF 'cd /opt/restichub && docker compose logs' "$work/migrate-int.log" ||
  fail "the log hint needs a directory that the rollback removes"
grep -qF 'No migration ran: the schema version is still 58' "$work/migrate-int.log" || fail "the hint does not say that no migration ran"
run_rollback /var/backups/restichub
expect_running "$IMAGE_A"
as_root test -d "$clone/.git" || fail "the rollback did not put back the clone"
volume_created=$(as_root docker volume inspect --format '{{.CreatedAt}}' restichub_pgdata)

step "Move the git clone installation to 0.0.2-smoke.2 after the rollback"
# The stack of the clone applied 0058 again; the move must apply a migration.
undo_last_migration
args=(--migrate-from-git "$clone" --version 0.0.2-smoke.2)
run "$work/migrate.log" || { cat "$work/migrate.log"; fail "the move failed"; }
cat "$work/migrate.log"
expect_running "$IMAGE_B"
[ "$(sha_of "$kek")" = "$old_kek_sha" ] || fail "the KEK copy differs from the KEK of the clone"
[ "$(mode_of "$kek")" = '400 10001' ] || fail "the KEK copy is not 400 uid 10001"
[ "$(mode_of "$config/.env")" = '600 0' ] || fail "the environment file copy is not 600 root"
kept=$(as_root find /opt -maxdepth 1 -name 'restichub.git-backup-*' | sort | tail -n 1)
[ -n "$kept" ] || fail "the clone was not kept"
as_root test -f "$kept/deploy/reference/secrets/restichub_kek" || fail "the KEK of the clone was deleted"
[ ! -e /opt/restichub/.git ] || fail "/opt/restichub still holds the clone"
[ "$(as_root docker volume inspect --format '{{.CreatedAt}}' restichub_pgdata)" = "$volume_created" ] ||
  fail "the database volume is not the volume of the clone"
[ "$(sql 'SELECT count(*) FROM users')" -gt 0 ] || fail "the database of the move has no user"
grep -qF 'The schema version: 57 -> 58.' "$work/migrate.log" || fail "the move does not report the schema version"
as_root grep -qF 'The restore loses every change after' "$(latest_backup /var/backups/restichub)ROLLBACK.txt" ||
  fail "ROLLBACK.txt does not say that a restore loses the changes after the move"
[ "$(pg_dir)" = /opt/restichub ] || fail "PostgreSQL still comes from $(pg_dir)"

step "Upgrade the moved server to 0.0.5-smoke.5"
args=(--upgrade --version 0.0.5-smoke.5)
run "$work/upgrade-moved-server.log" || { cat "$work/upgrade-moved-server.log"; fail "the upgrade after the move failed"; }
cat "$work/upgrade-moved-server.log"
expect_running "$IMAGE_A"
[ "$(pg_dir)" = /opt/restichub ] || fail "PostgreSQL comes from $(pg_dir) after the upgrade"

echo
echo "The control plane install smoke test passed."

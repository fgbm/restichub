#!/usr/bin/env bash
# The control plane install smoke test (ADR-0056, T38). It runs deploy/reference/install.sh as
# root through its test seam: a local file server for the release files and cosign, a local
# registry for the image, and a test key instead of the keyless identity.
#
# Usage: cp-install-smoke.sh
#   COSIGN    the cosign binary of the release workflow pin
#   IMAGE_A   the first control plane image, localhost:5000/fgbm/restichub-cp@sha256:<digest>
#   IMAGE_B   a second image with another digest in the same repository
#   OLD_TAG   the tag whose deploy/reference is the fake git clone (default v0.1.0-rc.9)
#
# The seam cannot check the keyless signature of the release workflow. The `verify` job of
# release.yml checks the published files with the commands of the guide, and a release
# candidate runs on a clean server before the release (CONTRIBUTING.md).
set -euo pipefail

: "${COSIGN:?}" "${IMAGE_A:?}" "${IMAGE_B:?}"
OLD_TAG=${OLD_TAG:-v0.1.0-rc.9}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/cp-install-smoke.XXXXXX")
srv=$work/srv
port=8099
repo_name=localhost:5000/fgbm/restichub-cp
config=/etc/restichub
kek=$config/secrets/restichub_kek
sudo=()
[ "$(id -u)" -eq 0 ] || sudo=(sudo)

fail() { echo "FAIL: $*" >&2; exit 1; }
step() { echo; echo "=== $*"; }

server_pid=''
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
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
for ref in "$IMAGE_A" "$IMAGE_B"; do
  "$COSIGN" sign --yes --key "$work/test.key" --tlog-upload=false --use-signing-config=false "$ref" 2>/dev/null
done
release() {
  local version=$1 image=$2 dir=$srv/v$1
  "$root/packaging/make-deploy-archive.sh" "$version" "$image" "$dir" >/dev/null
  "$COSIGN" sign-blob --yes --key "$work/test.key" --use-signing-config=false --tlog-upload=false \
    --bundle "$dir/restichub-deploy-$version.tar.gz.sigstore.json" "$dir/restichub-deploy-$version.tar.gz" 2>/dev/null
}
release 0.0.1-smoke.1 "$IMAGE_A"
release 0.0.2-smoke.2 "$IMAGE_B"
release 0.0.3-smoke.3 "$IMAGE_A"
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

step "The published install.sh holds the version and no test seam"
published=$srv/v0.0.1-smoke.1/install.sh
grep -qx "release_version='0.0.1-smoke.1'" "$published" || fail "the published script has no version"
! grep -q 'RESTICHUB_TEST_' "$published" || fail "the published script holds the test seam"
shellcheck "$root/deploy/reference/install.sh" "$published"
bash "$published" --help >/dev/null

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
  "${sudo[@]}" env "${seam[@]}" "$@" bash "$root/deploy/reference/install.sh" "${args[@]}" >"$log" 2>&1
}
expect_refusal() {
  local name=$1 log=$work/$1.log text=$2
  shift 2
  if run "$log" "$@"; then cat "$log"; fail "the run $name did not refuse"; fi
  grep -qF "$text" "$log" || { cat "$log"; fail "the refusal does not say: $text"; }
  [ ! -e "$config" ] && [ ! -e /opt/restichub ] || fail "a refused run changed $config or /opt/restichub"
  echo "refused as expected: $(grep '^error:' "$log" | tail -n 1)"
}
cp_container() { "${sudo[@]}" docker ps -q --filter label=com.docker.compose.project=restichub --filter label=com.docker.compose.service=cp; }
expect_running() {
  local cid
  cid=$(cp_container)
  [ "$("${sudo[@]}" docker inspect --format '{{.Config.Image}}' "$cid")" = "$1" ] || fail "cp does not run $1"
  [ "$("${sudo[@]}" docker inspect --format '{{.State.Health.Status}}' "$cid")" = healthy ] || fail "cp is not healthy"
}
mode_of() { "${sudo[@]}" stat -c '%a %u' "$1"; }
sha_of() { "${sudo[@]}" sha256sum "$1" | cut -d' ' -f1; }

step "A run without a terminal and without --kek-saved refuses"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com)
expect_refusal no-tty 'no terminal for the confirmation' setsid -w

step "A cosign download with a wrong SHA-256 stops the run"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
expect_refusal bad-cosign 'SHA-256 of the cosign download' RESTICHUB_TEST_COSIGN_BASE_URL="http://127.0.0.1:$port/badcosign"

step "A changed release archive stops the run"
args=(--version 0.0.3-smoke.3 --domain localhost --email admin@example.com --kek-saved)
expect_refusal tampered 'the signature of restichub-deploy-0.0.3-smoke.3.tar.gz is not valid'

step "Install 0.0.1-smoke.1"
args=(--version 0.0.1-smoke.1 --domain localhost --email admin@example.com --kek-saved)
run "$work/install.log" || { cat "$work/install.log"; fail "the install failed"; }
cat "$work/install.log"
expect_running "$IMAGE_A"
[ "$(mode_of "$config/.env")" = '600 0' ] || fail "the environment file mode is not 600 root"
[ "$(mode_of "$kek")" = '400 10001' ] || fail "the KEK mode is not 400 uid 10001"
[ "$(readlink /opt/restichub/.env)" = "$config/.env" ] || fail "/opt/restichub/.env is not the link"
! grep -qF "$("${sudo[@]}" cat "$kek")" "$work/install.log" || fail "the output holds the KEK"
grep -qF "starts with $(sha_of "$kek" | cut -c1-8)" "$work/install.log" || fail "the output has no KEK fingerprint"
(cd /opt/restichub && "${sudo[@]}" docker compose logs cp 2>&1 | grep setup_token >/dev/null) ||
  fail "no setup token in the log"
kek_sha=$(sha_of "$kek")

step "A second install refuses an installed server"
if run "$work/again.log"; then fail "the second install did not refuse"; fi
grep -qF 'so this server is installed' "$work/again.log" || { cat "$work/again.log"; fail "wrong refusal"; }

step "Upgrade to 0.0.2-smoke.2"
args=(--upgrade --version 0.0.2-smoke.2)
run "$work/upgrade.log" || { cat "$work/upgrade.log"; fail "the upgrade failed"; }
cat "$work/upgrade.log"
expect_running "$IMAGE_B"
[ "$(sha_of "$kek")" = "$kek_sha" ] || fail "the upgrade changed the KEK"
backup=$("${sudo[@]}" find /var/backups/restichub -mindepth 1 -maxdepth 1 -type d | head -n 1)
for file in restichub.env release/compose.yaml; do
  "${sudo[@]}" test -f "$backup/$file" || fail "the backup has no $file"
done
"${sudo[@]}" sh -c "ls $backup/restichub-*.dump" >/dev/null || fail "the backup has no dump"
"${sudo[@]}" grep -qx "RESTICHUB_IMAGE=$IMAGE_B" "$config/.env" || fail "the environment file has not the new image"
"${sudo[@]}" grep -qx "RESTICHUB_IMAGE=$IMAGE_A" "$backup/restichub.env" || fail "the backup has not the old image"

step "Remove the installation"
(cd /opt/restichub && "${sudo[@]}" docker compose down -v)
"${sudo[@]}" rm -rf "$config" /opt/restichub /var/backups/restichub

step "Start a git clone installation of $OLD_TAG"
"${sudo[@]}" mkdir -p /opt/restichub
git -C "$root" archive "$OLD_TAG" deploy/reference crates/restichub-db/sql | "${sudo[@]}" tar -x -C /opt/restichub
"${sudo[@]}" git init -q /opt/restichub
(
  cd /opt/restichub/deploy/reference
  "${sudo[@]}" ./init-env.sh localhost admin@example.com "$IMAGE_A" >/dev/null
  "${sudo[@]}" docker compose up -d --quiet-pull
)
old_kek=/opt/restichub/deploy/reference/secrets/restichub_kek
old_kek_sha=$(sha_of "$old_kek")
old_kek_mode=$(mode_of "$old_kek")
volume_created=$("${sudo[@]}" docker volume inspect --format '{{.CreatedAt}}' restichub_pgdata)
expect_running "$IMAGE_A"

step "Move the git clone installation to 0.0.2-smoke.2"
args=(--migrate-from-git /opt/restichub --version 0.0.2-smoke.2)
run "$work/migrate.log" || { cat "$work/migrate.log"; fail "the move failed"; }
cat "$work/migrate.log"
expect_running "$IMAGE_B"
[ "$(sha_of "$kek")" = "$old_kek_sha" ] || fail "the KEK copy differs from the KEK of the clone"
[ "$(mode_of "$kek")" = "$old_kek_mode" ] || fail "the KEK copy has another mode or owner"
clone=$("${sudo[@]}" find /opt -maxdepth 1 -name 'restichub.git-backup-*' | head -n 1)
[ -n "$clone" ] || fail "the clone was not kept"
"${sudo[@]}" test -f "$clone/deploy/reference/secrets/restichub_kek" || fail "the KEK of the clone was deleted"
[ ! -e /opt/restichub/.git ] || fail "/opt/restichub still holds the clone"
[ "$("${sudo[@]}" docker volume inspect --format '{{.CreatedAt}}' restichub_pgdata)" = "$volume_created" ] ||
  fail "the database volume is not the volume of the clone"

echo
echo "The control plane install smoke test passed."

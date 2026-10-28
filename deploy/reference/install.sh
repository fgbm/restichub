#!/usr/bin/env bash
# Install, upgrade or move the ResticHub control plane from a signed release (ADR-0056).
#
# The script verifies every file that it uses: cosign by a fixed SHA-256, the release
# archive and the control plane image by the keyless signature of the release workflow
# at the tag of the version. It has no option that skips a check.
set -euo pipefail

# packaging/make-deploy-archive.sh writes the version of the release here.
release_version=''

readonly repo=fgbm/restichub
readonly install_dir=/opt/restichub
readonly config_dir=/etc/restichub
readonly env_file=$config_dir/.env
readonly kek_file=$config_dir/secrets/restichub_kek
readonly kek_uid=10001
readonly project=restichub
# The same pins as COSIGN_URL and COSIGN_SHA256 in .github/workflows/release.yml.
readonly cosign_version=v3.0.6
readonly cosign_sha256_amd64=c956e5dfcac53d52bcf058360d579472f0c1d2d9b69f55209e256fe7783f4c74
readonly cosign_sha256_arm64=bedac92e8c3729864e13d4a17048007cfafa79d5deca993a43a90ffe018ef2b8
readonly oidc_issuer=https://token.actions.githubusercontent.com

releases_url="https://github.com/$repo/releases/download"
cosign_base_url="https://github.com/sigstore/cosign/releases/download/$cosign_version"
image_repo="ghcr.io/$repo-cp"
fetch_proto='=https'

check_args=()
set_check_args() {
  check_args=(
    --certificate-identity "https://github.com/$repo/.github/workflows/release.yml@refs/tags/v$version"
    --certificate-oidc-issuer "$oidc_issuer"
  )
}

# BEGIN TEST SEAM
# Only the CI smoke test sets these variables. packaging/make-deploy-archive.sh removes this
# block from the published script, so a real run never reads them. The SHA-256 of cosign
# stays checked.
if [ -n "${RESTICHUB_TEST_RELEASES_URL:-}" ]; then
  releases_url=$RESTICHUB_TEST_RELEASES_URL
  cosign_base_url=${RESTICHUB_TEST_COSIGN_BASE_URL:?}
  image_repo=${RESTICHUB_TEST_IMAGE_REPO:?}
  fetch_proto='=http,https'
  set_check_args() {
    check_args=(--key "${RESTICHUB_TEST_COSIGN_KEY:?}" --insecure-ignore-tlog=true)
  }
fi
# END TEST SEAM

usage() {
  cat <<EOF
Install, upgrade or move the ResticHub control plane from a signed release.

Usage:
  install.sh --domain DOMAIN --email EMAIL [--kek-saved] [--version VERSION]
  install.sh --upgrade [--version VERSION] [--backup-dir DIR]
  install.sh --migrate-from-git [DIR] [--version VERSION] [--backup-dir DIR]

Modes:
  (no mode)              Install a new server. The script refuses a server that has
                         $env_file.
  --upgrade              Upgrade an installed server. The script saves the environment
                         file, the old release files and a database dump first.
  --migrate-from-git     Move a server that was installed with git clone (default DIR:
                         $install_dir) to the layout of this script.

Options:
  --domain DOMAIN        The domain name of the control plane.
  --email EMAIL          The email address for the certificate of the domain.
  --version VERSION      The release to install, for example 1.2.0. The default is the
                         version of this script.
  --backup-dir DIR       The directory for the backups (default /var/backups/restichub).
  --kek-saved            For a run without a terminal: you confirm that you copy the
                         key encryption key and the environment file outside the server.
  -h, --help             Show this help.

Files:
  $install_dir           The release files. Each upgrade replaces them.
  $env_file      The environment file with the passwords (mode 600).
  $kek_file  The key encryption key (mode 400, uid $kek_uid).
EOF
}

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

mode=install
domain=''
email=''
version=''
kek_saved=false
git_dir=$install_dir
backup_root=/var/backups/restichub
work=''
rollback_hint=''

need_value() {
  if [ -z "${2:-}" ] || [ "${2#--}" != "$2" ]; then
    die "the option $1 needs a value (see --help)"
  fi
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --domain) need_value "$1" "${2:-}"; domain=$2; shift 2 ;;
      --email) need_value "$1" "${2:-}"; email=$2; shift 2 ;;
      --version) need_value "$1" "${2:-}"; version=${2#v}; shift 2 ;;
      --backup-dir) need_value "$1" "${2:-}"; backup_root=$2; shift 2 ;;
      --kek-saved) kek_saved=true; shift ;;
      --upgrade) mode=upgrade; shift ;;
      --migrate-from-git)
        mode=migrate
        if [ -n "${2:-}" ] && [ "${2#--}" = "$2" ]; then git_dir=${2%/}; shift; fi
        [ "${git_dir#/}" != "$git_dir" ] || die "give the directory of the git clone as an absolute path"
        shift ;;
      -h | --help) usage; exit 0 ;;
      *) die "unknown option: $1 (see --help)" ;;
    esac
  done
  version=${version:-$release_version}
  [ -n "$version" ] || die "this copy of the script has no release version; give --version"
  local semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
  [[ "$version" =~ $semver ]] || die "the version $version is not a release version, for example 1.2.0"
  if [ "$mode" = install ]; then
    if [ -z "$domain" ] || [ -z "$email" ]; then die "give --domain and --email (see --help)"; fi
    [[ "$domain" =~ ^[A-Za-z0-9.-]+$ ]] || die "the domain $domain is not a host name"
    [[ "$email" =~ ^[^[:space:]@]+@[^[:space:]@]+$ ]] || die "the email address $email is not valid"
  elif [ -n "$domain" ] || [ -n "$email" ] || $kek_saved; then
    die "--domain, --email and --kek-saved apply only to a new installation"
  fi
}

on_exit() {
  local status=$?
  if [ "$status" -ne 0 ] && [ -n "$rollback_hint" ]; then
    printf '\n%s\n' "$rollback_hint" >&2
  fi
  if [ -n "$work" ]; then rm -rf "$work"; fi
  exit "$status"
}

check_host() {
  [ "$(id -u)" -eq 0 ] || die "run the script as root, for example with sudo"
  local tool
  for tool in curl tar gzip sha256sum awk sed mktemp install stat; do
    command -v "$tool" >/dev/null || die "the command $tool is missing; install it and run the script again"
  done
  docker version --format '{{.Server.Version}}' >/dev/null 2>&1 ||
    die "Docker Engine does not answer; install Docker Engine and start it"
  local compose_version
  compose_version=$(docker compose version --short 2>/dev/null) ||
    die "the Docker Compose plugin is missing; install the package docker-compose-plugin"
  compose_version=${compose_version#v}
  [ "${compose_version%%.*}" -ge 2 ] 2>/dev/null ||
    die "Docker Compose $compose_version is too old; install Docker Compose version 2 or later"
  case "$(uname -m)" in
    x86_64) arch=amd64 ;;
    aarch64 | arm64) arch=arm64 ;;
    *) die "the architecture $(uname -m) has no control plane image" ;;
  esac
}

check_ports() {
  if ! command -v ss >/dev/null; then
    warn "the command ss is missing; the script does not check the ports 80, 443 and 8443"
    return
  fi
  local port
  for port in 80 443 8443; do
    if [ -n "$(ss -Hltn "sport = :$port")" ]; then
      die "another program listens on port $port; stop it and run the script again"
    fi
  done
}

check_install_target() {
  [ ! -e "$env_file" ] ||
    die "$env_file exists, so this server is installed; use --upgrade"
  if [ -e "$install_dir/.git" ] || [ -e "$install_dir/deploy/reference/compose.yaml" ]; then
    die "$install_dir holds a git clone of ResticHub; use --migrate-from-git"
  fi
  [ ! -e "$install_dir" ] || [ -z "$(ls -A "$install_dir")" ] ||
    die "$install_dir is not empty; move it away and run the script again"
  if ! $kek_saved && ! { : </dev/tty; } 2>/dev/null; then
    die "the script has no terminal for the confirmation of the key copy; give --kek-saved"
  fi
}

check_upgrade_target() {
  if [ ! -f "$env_file" ] || [ ! -f "$kek_file" ]; then
    die "$env_file or $kek_file is missing; install the server first"
  fi
  if [ ! -f "$install_dir/compose.yaml" ] || [ ! -L "$install_dir/.env" ]; then
    die "$install_dir does not hold the files of this script; for a git clone, use --migrate-from-git"
  fi
}

check_migrate_target() {
  src_env=$git_dir/deploy/reference/.env
  src_kek=$git_dir/deploy/reference/secrets/restichub_kek
  [ -f "$src_env" ] || die "$src_env is missing; give the directory of the git clone"
  [ -f "$src_kek" ] || die "$src_kek is missing; this script moves only a KEK file"
  if [ -e "$env_file" ] || [ -e "$kek_file" ]; then
    # A run that stopped after the copy can run again with the same copies.
    if ! { same_file "$src_env" "$env_file" && same_file "$src_kek" "$kek_file"; }; then
      die "$config_dir holds other files than the clone; compare them by hand"
    fi
  fi
  if [ "$git_dir" != "$install_dir" ] && [ -e "$install_dir" ]; then
    die "$install_dir exists; move it away and run the script again"
  fi
}

same_file() {
  [ -f "$2" ] && [ "$(sha256sum <"$1")" = "$(sha256sum <"$2")" ] &&
    [ "$(stat -c '%a %u %g' "$1")" = "$(stat -c '%a %u %g' "$2")" ]
}

fetch() {
  curl --fail --silent --show-error --location --retry 3 --proto "$fetch_proto" --output "$2" "$1" ||
    die "the download of $1 failed"
}

get_cosign() {
  local sha
  case "$arch" in
    amd64) sha=$cosign_sha256_amd64 ;;
    arm64) sha=$cosign_sha256_arm64 ;;
  esac
  say "Downloading cosign $cosign_version"
  fetch "$cosign_base_url/cosign-linux-$arch" "$work/cosign"
  (cd "$work" && printf '%s  cosign\n' "$sha" | sha256sum --check --status) ||
    die "the SHA-256 of the cosign download is not $sha; the file is incomplete or changed"
  chmod 0755 "$work/cosign"
  cosign=$work/cosign
  "$cosign" version >/dev/null 2>&1 || die "cosign $cosign_version does not run on this server"
}

get_release() {
  local name=restichub-deploy-$version
  local archive=$work/$name.tar.gz
  say "Downloading and verifying the release files of $version"
  fetch "$releases_url/v$version/$name.tar.gz" "$archive"
  fetch "$releases_url/v$version/$name.tar.gz.sigstore.json" "$archive.sigstore.json"
  set_check_args
  "$cosign" verify-blob --bundle "$archive.sigstore.json" "${check_args[@]}" "$archive" \
    >"$work/cosign.log" 2>&1 || {
    cat "$work/cosign.log" >&2
    die "the signature of $name.tar.gz is not valid for the release v$version"
  }
  local entry
  while IFS= read -r entry; do
    case "$entry" in
      "$name"/*/* | */../* | */..) die "the archive holds an unexpected path: $entry" ;;
      "$name"/ | "$name"/*) ;;
      *) die "the archive holds an unexpected path: $entry" ;;
    esac
  done < <(tar -tzf "$archive")
  local types
  types=$(tar -tvzf "$archive" | cut -c1 | tr -d '\n')
  if [ -n "${types//[-d]/}" ]; then
    die "the archive holds a link or a special file"
  fi
  mkdir "$work/release"
  tar -xzf "$archive" -C "$work/release" --strip-components=1 --no-same-owner
  local file
  for file in compose.yaml Caddyfile bootstrap-roles.sql IMAGE; do
    [ -f "$work/release/$file" ] || die "the archive has no $file"
  done
  image_ref=$(head -n 1 "$work/release/IMAGE")
  local digest=${image_ref#"$image_repo"@sha256:}
  if [ "$digest" = "$image_ref" ] || ! [[ "$digest" =~ ^[0-9a-f]{64}$ ]]; then
    die "the archive names the image $image_ref, not $image_repo with a digest"
  fi
}

verify_image() {
  say "Verifying the image $image_ref"
  "$cosign" verify "${check_args[@]}" "$image_ref" >"$work/cosign.log" 2>&1 || {
    cat "$work/cosign.log" >&2
    die "the signature of the image $image_ref is not valid for the release v$version"
  }
  docker pull --quiet "$image_ref" >/dev/null
}

random_secret() {
  head -c 32 /dev/urandom | base64
}

write_config() {
  install -d -m 0700 -o root -g root "$config_dir" "$config_dir/secrets"
  (
    umask 077
    set -o noclobber
    random_secret >"$kek_file"
    cat >"$env_file" <<EOF
POSTGRES_USER=restichub
POSTGRES_PASSWORD=$(random_secret)
POSTGRES_DB=restichub
RESTICHUB_OWNER_PASSWORD=$(random_secret)
RESTICHUB_RUNTIME_PASSWORD=$(random_secret)
RESTICHUB_DOMAIN=$domain
RESTICHUB_ACME_EMAIL=$email
RESTICHUB_IMAGE=$image_ref
RESTICHUB_AGENT_ENDPOINT=https://$domain:8443
RESTICHUB_TENANCY_MODE=single
RESTICHUB_TRUSTED_PROXIES=172.28.0.0/24
EOF
  )
  # The control plane refuses a key file that others can read.
  chmod 0400 "$kek_file"
  chown "$kek_uid:$kek_uid" "$kek_file"
}

copy_config() {
  install -d -m 0700 -o root -g root "$config_dir" "$config_dir/secrets"
  if [ ! -e "$env_file" ]; then
    cp -p "$src_env" "$env_file"
    cp -p "$src_kek" "$kek_file"
  fi
  if ! { same_file "$src_env" "$env_file" && same_file "$src_kek" "$kek_file"; }; then
    die "the copies in $config_dir differ from the files of the clone"
  fi
  say "Copied the environment file and the KEK to $config_dir; the SHA-256, the mode and the owner match"
}

env_value() {
  awk -v key="$1" 'index($0, key "=") == 1 { print substr($0, length(key) + 2); exit }' "$2"
}

set_env_image() {
  local tmp
  tmp=$(mktemp "$config_dir/.env.XXXXXX")
  awk -v ref="$image_ref" '
    /^RESTICHUB_IMAGE=/ { if (!done) print "RESTICHUB_IMAGE=" ref; done = 1; next }
    { print }
    END { if (!done) print "RESTICHUB_IMAGE=" ref }
  ' "$env_file" >"$tmp"
  chmod --reference="$env_file" "$tmp"
  chown --reference="$env_file" "$tmp"
  mv "$tmp" "$env_file"
}

make_backup_dir() {
  backup_dir=$backup_root/$stamp
  install -d -m 0700 "$backup_root"
  install -d -m 0700 "$backup_dir"
}

# $1 is the environment file of the running stack.
dump_database() {
  local cid user db dump=$backup_dir/restichub-$stamp.dump
  cid=$(docker ps -q --filter "label=com.docker.compose.project=$project" \
    --filter label=com.docker.compose.service=postgres --filter status=running | head -n 1)
  [ -n "$cid" ] || die "the PostgreSQL container of the project $project does not run; start the stack first"
  user=$(env_value POSTGRES_USER "$1")
  db=$(env_value POSTGRES_DB "$1")
  say "Writing the database dump to $dump"
  # The superuser makes the dump, because the tables have row-level security.
  docker exec "$cid" pg_dump -U "${user:-restichub}" -Fc "${db:-restichub}" >"$dump" ||
    die "pg_dump failed"
  [ -s "$dump" ] || die "the dump $dump is empty"
  docker exec -i "$cid" pg_restore --list <"$dump" >/dev/null ||
    die "pg_restore cannot read the dump $dump"
  dump_file=$dump
}

place_release() {
  local staged old
  staged=$(mktemp -d "$(dirname "$install_dir")/.restichub-new.XXXXXX")
  cp -R "$work/release/." "$staged/"
  ln -s "$env_file" "$staged/.env"
  chown -R root:root "$staged"
  chmod 0755 "$staged"
  if [ -e "$install_dir" ]; then
    old=$(dirname "$install_dir")/.restichub-old.$$
    mv "$install_dir" "$old"
    mv "$staged" "$install_dir"
    rm -rf "$old"
  else
    mv "$staged" "$install_dir"
  fi
}

compose() {
  (cd "$install_dir" && docker compose "$@")
}

start_stack() {
  say "Starting the stack"
  compose pull --quiet
  compose up -d ||
    die "the stack did not start; read the log with: cd $install_dir && docker compose logs migrate cp"
  local cid status='' i
  for i in $(seq 1 60); do
    cid=$(compose ps -q cp)
    if [ -n "$cid" ]; then
      status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$cid")
      [ "$status" = healthy ] && break
    fi
    [ "$i" -lt 60 ] || die "the control plane is not healthy after 5 minutes; read the log with: cd $install_dir && docker compose logs cp"
    sleep 5
  done
  local running expected
  running=$(docker inspect --format '{{.Config.Image}}' "$cid")
  [ "$running" = "$image_ref" ] || die "the control plane runs $running, not the verified $image_ref"
  running=$(docker inspect --format '{{.Image}}' "$cid")
  expected=$(docker image inspect --format '{{.Id}}' "$image_ref")
  [ "$running" = "$expected" ] || die "the image of the running control plane is not the verified image"
  say "The control plane is healthy and runs the verified image $image_ref"
}

kek_fingerprint() {
  sha256sum "$kek_file" | cut -c1-8
}

confirm_kek_saved() {
  cat <<EOF

The key encryption key (KEK) is in $kek_file.
Its SHA-256 starts with $(kek_fingerprint). The environment file is $env_file.
Without the KEK, nobody can decrypt the secrets in the database.
Copy both files to a password manager or a safe outside this server now.
EOF
  if $kek_saved; then
    say "You gave --kek-saved: you confirmed that you copy both files outside the server."
    return
  fi
  local answer=''
  while [ "$answer" != saved ]; do
    printf 'Type "saved" when the copy is outside the server: ' >/dev/tty
    IFS= read -r answer </dev/tty || die "no answer from the terminal; copy the files and remember them"
  done
}

run_install() {
  check_install_target
  check_ports
  get_cosign
  get_release
  verify_image
  rollback_hint="The installation did not finish. The database holds no data yet. To start again, run:
  cd $install_dir && docker compose down -v; rm -rf $install_dir $config_dir
Then run this script again."
  write_config
  place_release
  start_stack
  rollback_hint=''
  confirm_kek_saved
  cat <<EOF

The installation is complete. Read the one-time setup token with:
  cd $install_dir && docker compose logs cp | grep setup_token
Then open https://$domain and create the first administrator.
EOF
}

run_upgrade() {
  check_upgrade_target
  get_cosign
  get_release
  verify_image
  make_backup_dir
  cp -p "$env_file" "$backup_dir/restichub.env"
  cp -R "$install_dir" "$backup_dir/release"
  say "Saved the environment file and the release files to $backup_dir"
  dump_database "$env_file"
  rollback_hint="The upgrade did not finish. To start the release that ran before, run:
  cd $install_dir && docker compose down
  cp -p $backup_dir/restichub.env $env_file
  rm -rf $install_dir && cp -R $backup_dir/release $install_dir
  cd $install_dir && docker compose up -d
If the migrate service of the new release ran, also restore the dump $dump_file.
See \"Go back to the old release\" in the upgrade guide."
  place_release
  set_env_image
  start_stack
  rollback_hint=''
  say "The upgrade to $version is complete. The backup is in $backup_dir."
}

run_migrate() {
  check_migrate_target
  get_cosign
  get_release
  verify_image
  copy_config
  make_backup_dir
  dump_database "$src_env"
  local git_backup=$git_dir.git-backup-$stamp
  mv "$git_dir" "$git_backup"
  say "Moved the git clone to $git_backup"
  rollback_hint="The move did not finish. To start the stack of the git clone again, run:
  cd $install_dir && docker compose down; rm -rf $install_dir
  mv $git_backup $git_dir
  cd $git_dir/deploy/reference && docker compose up -d
If the migrate service of the new release ran, also restore the dump $dump_file."
  place_release
  set_env_image
  start_stack
  rollback_hint=''
  cat <<EOF

The server uses the files of this script now. The KEK is in $kek_file.
Its SHA-256 starts with $(kek_fingerprint), the same as the KEK of the clone.
The old clone is in $git_backup and still holds the KEK and the environment file.
Make sure that a copy of $kek_file and $env_file is outside the server.
Then delete the old clone: rm -rf $git_backup
EOF
}

main() {
  parse_args "$@"
  trap on_exit EXIT
  check_host
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  work=$(mktemp -d /var/tmp/restichub-install.XXXXXX)
  case "$mode" in
    install) run_install ;;
    upgrade) run_upgrade ;;
    migrate) run_migrate ;;
  esac
}

main "$@"

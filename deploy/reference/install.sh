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
# The one place of the KEK path: compose.yaml names the same file.
readonly config_dir=/etc/restichub
readonly env_file=$config_dir/.env
readonly kek_file=$config_dir/secrets/restichub_kek
readonly failed_marker=$config_dir/INSTALL-FAILED
readonly kek_uid=10001
readonly project=restichub
readonly pg_volume=${project}_pgdata
readonly pg_data=/var/lib/postgresql/data
readonly lock_file=/run/restichub-install.lock
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
  install.sh --upgrade [--version VERSION] [--backup-dir DIR] [--force-after-failure]
  install.sh --migrate-from-git [DIR] [--version VERSION] [--backup-dir DIR] [--force-after-failure]

Modes:
  (no mode)              Install a new server. The script refuses a server that has a
                         KEK, an environment file, the database volume or a container
                         of the project $project.
  --upgrade              Upgrade an installed server. The script saves the environment
                         file, the old release files and a database dump first.
  --migrate-from-git     Move a server that was installed with git clone (default DIR:
                         $install_dir) to the layout of this script.

Options:
  --domain DOMAIN        The domain name of the control plane.
  --email EMAIL          The email address for the certificate of the domain.
  --version VERSION      The release to install, for example 1.2.0. The default is the
                         version of this script.
  --backup-dir DIR       The absolute path of the parent directory for the backups
                         (default /var/backups/restichub). Each run makes a new
                         subdirectory. The script does not delete old backups.
  --force-after-failure  Run although an earlier run failed and left $failed_marker.
  --kek-saved            For a run without a terminal: you confirm that you copy the
                         key encryption key and the environment file outside the server.
  -h, --help             Show this help.

Files:
  $install_dir           The release files. Each upgrade replaces them.
  $env_file      The environment file with the passwords (mode 600).
  $kek_file  The key encryption key (mode 400, uid $kek_uid).

Run the script in tmux or screen: a lost SSH connection stops it. When a run fails after a
change, the backup directory holds ROLLBACK.txt with the commands that go back.
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
force_after_failure=false
git_dir=$install_dir
backup_root=/var/backups/restichub
work=''
stamp=''
backup_dir=''
dump_file=''
git_backup=''
pg_user=restichub
pg_db=restichub
schema_before=''
db_id_before=''
db_mount_before=''
users_before=0
# The state of the run that on_exit reads to write the rollback commands.
created=()
changing=false
finished=false
placed=false
db_mismatch=false

need_value() {
  if [ -z "${2:-}" ] || [ "${2#--}" != "$2" ]; then
    die "the option $1 needs a value (see --help)"
  fi
}

# A host name of labels from 1 to 63 letters, digits and hyphens, with no hyphen at the
# ends of a label. It also keeps quotes and `$` out of the environment file.
valid_domain() {
  local label='[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?'
  [ "${#1}" -le 253 ] && [[ "$1" =~ ^$label(\.$label)*$ ]]
}

inside() {
  case "$1/" in "$2"/*) return 0 ;; esac
  return 1
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --domain) need_value "$1" "${2:-}"; domain=$2; shift 2 ;;
      --email) need_value "$1" "${2:-}"; email=$2; shift 2 ;;
      --version) need_value "$1" "${2:-}"; version=${2#v}; shift 2 ;;
      --backup-dir) need_value "$1" "${2:-}"; backup_root=${2%/}; shift 2 ;;
      --kek-saved) kek_saved=true; shift ;;
      --force-after-failure) force_after_failure=true; shift ;;
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
    valid_domain "$domain" || die "the domain $domain is not a host name"
    [[ "$email" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+$ ]] || die "the email address $email is not valid"
    $force_after_failure && die "--force-after-failure applies only to --upgrade and --migrate-from-git"
  elif [ -n "$domain" ] || [ -n "$email" ] || $kek_saved; then
    die "--domain, --email and --kek-saved apply only to a new installation"
  fi
  [ "${backup_root#/}" != "$backup_root" ] || die "give --backup-dir as an absolute path"
  [[ "$backup_root" != *[[:space:]]* ]] || die "the backup directory $backup_root holds a space"
  local place
  for place in "$install_dir" "$git_dir" "$config_dir"; do
    if inside "$backup_root" "$place"; then
      die "the backup directory $backup_root is inside $place, which the run replaces or moves"
    fi
  done
}

# The text that a failed run prints and writes to ROLLBACK.txt. $1 is the schema version
# after the failure, or an empty value when the script cannot read it.
failure_text() {
  local schema_now=$1
  case "$mode" in
    install) install_failure_text ;;
    upgrade | migrate)
      if ! $changing && ! $finished; then
        [ -z "$backup_dir" ] || say "The run stopped before it changed the stack. The old stack still runs; no step is necessary."
        return 0
      fi
      if $finished; then
        say "The run of $mode to $version finished. To go back to the stack that ran before it, use the commands below."
      elif [ "$mode" = upgrade ]; then
        say "The upgrade to $version did not finish."
      else
        say "The move to $version did not finish."
      fi
      say "The backup of the run is in $backup_dir, with the dump $dump_file."
      local restore=true
      if $db_mismatch; then
        say "The new stack found another database, and the script stopped it before a migration."
        say "The database of the old stack did not change. Do not restore the dump."
        restore=false
      elif [ -n "$schema_now" ] && [ "$schema_now" = "$schema_before" ] && ! $finished; then
        say "No migration ran: the schema version is still $schema_before. The commands below are enough; do not restore the dump."
        restore=false
      elif [ -n "$schema_now" ] && [ "$schema_now" = "$schema_before" ]; then
        say "No migration ran: the schema version is still $schema_before. Do not restore the dump: the changes after the run stay."
        restore=false
      elif [ -n "$schema_now" ]; then
        say "The schema moved $schema_before -> $schema_now. The old release cannot use the new schema: the commands below restore the dump first."
      else
        say "The script cannot read the schema version now (before the run: $schema_before)."
        say "Read it with the first command below. If it is $schema_before, skip the commands marked (restore)."
        say "  docker exec \$(docker ps -q --filter label=com.docker.compose.project=$project --filter label=com.docker.compose.service=postgres) psql -U $pg_user -d $pg_db -Atc 'SELECT max(version) FROM _sqlx_migrations WHERE success'"
      fi
      say "Run these commands as root to start the stack that ran before:"
      if [ "$mode" = upgrade ]; then upgrade_commands "$restore"; else migrate_commands "$restore"; fi
      say "Then run the script again, or see \"Go back to the old release\" in the upgrade guide."
      ;;
  esac
}

install_failure_text() {
  [ "${#created[@]}" -gt 0 ] || return 0
  say "The installation did not finish. To remove what this run made, run as root:"
  # The volumes belong to this run: check_install_target refused a server with any volume
  # or container of the project.
  if $placed; then say "  cd $install_dir && docker compose down -v"; fi
  local path
  for path in "${created[@]}"; do say "  rm -rf $path"; done
  say "Then run this script again."
}

# $1 is the directory of the stack, $2 is true when the dump must be restored.
restore_commands() {
  $2 || return 0
  say "  cd $1"
  say "  docker volume rm $pg_volume  # (restore)"
  say "  docker compose up -d --wait postgres  # (restore)"
  say "  docker compose run --rm -T db-bootstrap  # (restore)"
  say "  docker compose exec -T postgres pg_restore -U $pg_user -d $pg_db --exit-on-error < $dump_file  # (restore)"
}

upgrade_commands() {
  say "  cd $install_dir && docker compose down"
  say "  cp -p $backup_dir/restichub.env $env_file"
  say "  cd / && rm -rf $install_dir && cp -a $backup_dir/release $install_dir"
  restore_commands "$install_dir" "$1"
  say "  cd $install_dir && docker compose up -d"
  say "  rm -f $failed_marker"
}

migrate_commands() {
  if $placed; then
    say "  cd $install_dir && docker compose down"
    say "  cd / && rm -rf $install_dir"
  fi
  say "  mv $git_backup $git_dir"
  restore_commands "$git_dir/deploy/reference" "$1"
  say "  cd $git_dir/deploy/reference && docker compose up -d"
  say "  rm -f $failed_marker"
}

write_rollback_file() {
  [ -n "$backup_dir" ] && [ -d "$backup_dir" ] || return 0
  failure_text "$1" >"$backup_dir/ROLLBACK.txt"
}

on_exit() {
  local status=$?
  set +e
  trap - EXIT
  if [ "$status" -ne 0 ]; then
    local schema_now=''
    if $changing && ! $db_mismatch; then schema_now=$(schema_version 2>/dev/null); fi
    # The file comes first: after a lost SSH connection the terminal is gone.
    write_rollback_file "$schema_now"
    if $changing && [ -d "$config_dir" ]; then
      printf 'The run of %s at %s failed. Its backup: %s\n' "$mode" "$stamp" "$backup_dir" >"$failed_marker"
    fi
    local text
    text=$(failure_text "$schema_now")
    if [ -n "$text" ]; then printf '\n%s\n' "$text" >&2; fi
    if [ -n "$backup_dir" ] && [ -f "$backup_dir/ROLLBACK.txt" ]; then
      printf 'The same text is in %s.\n' "$backup_dir/ROLLBACK.txt" >&2
    fi
  fi
  if [ -n "$work" ]; then rm -rf "$work"; fi
  exit "$status"
}

check_host() {
  [ "$(id -u)" -eq 0 ] || die "run the script as root, for example with sudo"
  local tool
  for tool in curl tar gzip sha256sum awk sed grep mktemp install stat seq base64 flock head cut tr \
    date uname df cmp ls cp mv ln rm mkdir chmod chown dirname; do
    command -v "$tool" >/dev/null || die "the command $tool is missing; install it and run the script again"
  done
  # The script uses options of GNU coreutils and GNU tar that BusyBox does not have. With
  # pipefail, `grep -q` must not end the pipe early: the writer then fails with SIGPIPE.
  sha256sum --version 2>/dev/null | grep 'GNU coreutils' >/dev/null ||
    die "sha256sum is not the GNU coreutils program; install coreutils and run the script again"
  stat --version 2>/dev/null | grep 'GNU coreutils' >/dev/null ||
    die "stat is not the GNU coreutils program; install coreutils and run the script again"
  tar --version 2>/dev/null | grep 'GNU tar' >/dev/null || die "tar is not GNU tar; install GNU tar and run the script again"
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
  # A COMPOSE_* variable of the shell can select other compose files or another project,
  # and so another database volume.
  local name
  while IFS= read -r name; do
    warn "the script ignores the variable $name of the shell"
    unset "$name"
  done < <(compgen -e | grep '^COMPOSE_' || true)
}

take_lock() {
  exec 9>"$lock_file"
  flock -n 9 || die "another run of install.sh holds $lock_file; wait for it to end"
}

make_work_dir() {
  local parent=${TMPDIR:-/var/tmp}
  [ "${parent#/}" != "$parent" ] || die "TMPDIR must be an absolute path"
  work=$(mktemp -d "$parent/restichub-install.XXXXXX")
  printf '#!/bin/sh\nexit 0\n' >"$work/exec-check"
  chmod 0700 "$work/exec-check"
  "$work/exec-check" 2>/dev/null ||
    die "$parent does not let programs run (noexec), so cosign cannot run; run the script with TMPDIR set to a directory on another file system"
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
  local found=()
  [ ! -e "$kek_file" ] || found+=("the KEK file $kek_file")
  [ ! -e "$env_file" ] || found+=("the environment file $env_file")
  if docker volume inspect "$pg_volume" >/dev/null 2>&1; then found+=("the database volume $pg_volume"); fi
  local volumes containers
  volumes=$(docker volume ls -q --filter "label=com.docker.compose.project=$project" | tr '\n' ' ')
  [ -z "$volumes" ] || found+=("the volumes of the project $project: $volumes")
  containers=$(docker ps -a --format '{{.Names}}' --filter "label=com.docker.compose.project=$project" | tr '\n' ' ')
  [ -z "$containers" ] || found+=("the containers of the project $project: $containers")
  if [ -e "$install_dir/.git" ] || [ -e "$install_dir/deploy/reference/compose.yaml" ]; then
    found+=("a git clone of ResticHub in $install_dir")
  elif [ -e "$install_dir" ] && [ -n "$(ls -A "$install_dir")" ]; then
    found+=("the directory $install_dir, which is not empty")
  fi
  if [ "${#found[@]}" -gt 0 ]; then
    printf 'error: this server is not empty. The script found:\n' >&2
    printf '  - %s\n' "${found[@]}" >&2
    die "the installation changed nothing. For an installed server use --upgrade; for a git clone use --migrate-from-git"
  fi
  if ! $kek_saved && ! { : </dev/tty; } 2>/dev/null; then
    die "the script has no terminal for the confirmation of the key copy; give --kek-saved"
  fi
}

check_failed_marker() {
  [ -e "$failed_marker" ] || return 0
  if $force_after_failure; then
    warn "an earlier run failed ($(cat "$failed_marker")); you gave --force-after-failure"
    return
  fi
  die "an earlier run failed: $(cat "$failed_marker"). Its backup holds the state before it, and ROLLBACK.txt there gives the commands that go back. Go back first and remove $failed_marker, or give --force-after-failure"
}

check_env_compose() {
  if grep -Eq '^[[:space:]]*(export[[:space:]]+)?COMPOSE_[A-Z_]+[[:space:]]*=' "$1"; then
    die "$1 sets a COMPOSE_ variable, so the stack can use other compose files or another project and database volume; remove the variable and run the script again"
  fi
}

check_override_files() {
  local file
  for file in compose.override.yaml compose.override.yml docker-compose.override.yaml docker-compose.override.yml; do
    [ ! -e "$1/$file" ] ||
      die "$1/$file exists. The new stack does not read it, so it can start on another database; move its changes into the stack by hand"
  done
}

check_upgrade_target() {
  if [ ! -f "$env_file" ] || [ ! -f "$kek_file" ]; then
    die "$env_file or $kek_file is missing; install the server first"
  fi
  if [ ! -f "$install_dir/compose.yaml" ] || [ ! -L "$install_dir/.env" ]; then
    die "$install_dir does not hold the files of this script; for a git clone, use --migrate-from-git"
  fi
  check_failed_marker
  check_env_compose "$env_file"
  check_override_files "$install_dir"
  read_db_names "$env_file"
}

check_migrate_target() {
  src_dir=$git_dir/deploy/reference
  src_env=$src_dir/.env
  src_kek=$src_dir/secrets/restichub_kek
  [ -f "$src_env" ] || die "$src_env is missing; give the directory of the git clone"
  [ -f "$src_kek" ] || die "$src_kek is missing; this script moves only a KEK file"
  local path
  for path in "$src_env" "$src_kek" "$src_dir/secrets"; do
    [ ! -L "$path" ] || die "$path is a symbolic link; replace it with the file that it points to and run the script again"
  done
  check_failed_marker
  check_env_compose "$src_env"
  check_override_files "$src_dir"
  # The deploy files must be the files of the commit. A changed compose.yaml can name
  # another volume, and the new stack then starts on an empty database.
  command -v git >/dev/null ||
    die "the command git is missing. Only git can compare deploy/reference of the clone with its commit; install git and run the script again"
  local changes
  changes=$(git -c "safe.directory=$git_dir" -C "$git_dir" status --porcelain --untracked-files=no -- deploy/reference) ||
    die "git cannot read the clone $git_dir"
  [ -z "$changes" ] ||
    die "deploy/reference of the clone has changes to the files of its commit: $(printf '%s' "$changes" | tr '\n' ' '). The new stack does not keep them; put the files back with git or move the changes by hand"
  if [ -e "$env_file" ] || [ -e "$kek_file" ]; then
    # A run that stopped after the copy, or that was rolled back, can run again with the
    # same copies. The image line is not compared: the failed run set its new image there.
    if ! { same_env "$src_env" "$env_file" && same_content "$src_kek" "$kek_file"; }; then
      die "$config_dir holds other files than the clone; compare them by hand, or move $config_dir away"
    fi
  fi
  if [ "$git_dir" != "$install_dir" ] && [ -e "$install_dir" ]; then
    die "$install_dir exists; move it away and run the script again"
  fi
  read_db_names "$src_env"
}

same_content() {
  [ -f "$2" ] && [ "$(sha256sum <"$1")" = "$(sha256sum <"$2")" ]
}

same_env() {
  [ -f "$2" ] &&
    [ "$(grep -v '^RESTICHUB_IMAGE=' "$1" | sha256sum)" = "$(grep -v '^RESTICHUB_IMAGE=' "$2" | sha256sum)" ]
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

record() { created+=("$1"); }

write_config() {
  if [ ! -e "$config_dir" ]; then
    record "$config_dir"
  else
    [ -e "$config_dir/secrets" ] || record "$config_dir/secrets"
    record "$env_file"
    record "$kek_file"
  fi
  install -d -m 0700 -o root -g root "$config_dir" "$config_dir/secrets"
  # Single quotes keep Compose from the interpolation of a `$` in a value.
  (
    umask 077
    set -o noclobber
    random_secret >"$kek_file"
    cat >"$env_file" <<EOF
POSTGRES_USER='restichub'
POSTGRES_PASSWORD='$(random_secret)'
POSTGRES_DB='restichub'
RESTICHUB_OWNER_PASSWORD='$(random_secret)'
RESTICHUB_RUNTIME_PASSWORD='$(random_secret)'
RESTICHUB_DOMAIN='$domain'
RESTICHUB_ACME_EMAIL='$email'
RESTICHUB_IMAGE='$image_ref'
RESTICHUB_AGENT_ENDPOINT='https://$domain:8443'
RESTICHUB_TENANCY_MODE='single'
RESTICHUB_TRUSTED_PROXIES='172.28.0.0/24'
EOF
  )
  set_config_modes
}

# The control plane refuses a key file that others can read.
set_config_modes() {
  chown root:root "$env_file"
  chmod 0600 "$env_file"
  chown "$kek_uid:$kek_uid" "$kek_file"
  chmod 0400 "$kek_file"
}

copy_config() {
  install -d -m 0700 -o root -g root "$config_dir" "$config_dir/secrets"
  if [ ! -e "$env_file" ]; then
    (
      umask 077
      cp "$src_env" "$env_file"
      cp "$src_kek" "$kek_file"
    )
  fi
  set_config_modes
  if ! { same_env "$src_env" "$env_file" && same_content "$src_kek" "$kek_file"; }; then
    die "the copies in $config_dir differ from the files of the clone"
  fi
  say "Copied the environment file and the KEK to $config_dir; the SHA-256 of each copy matches"
}

env_value() {
  awk -v key="$1" 'index($0, key "=") == 1 {
    value = substr($0, length(key) + 2)
    if (value ~ /^'\''.*'\''$/ || value ~ /^".*"$/) value = substr(value, 2, length(value) - 2)
    print value; exit
  }' "$2"
}

read_db_names() {
  pg_user=$(env_value POSTGRES_USER "$1")
  pg_db=$(env_value POSTGRES_DB "$1")
  pg_user=${pg_user:-restichub}
  pg_db=${pg_db:-restichub}
}

set_env_image() {
  local tmp
  tmp=$(mktemp "$config_dir/.env.XXXXXX")
  awk -v ref="$image_ref" '
    /^RESTICHUB_IMAGE=/ { if (!done) print "RESTICHUB_IMAGE='\''" ref "'\''"; done = 1; next }
    { print }
    END { if (!done) print "RESTICHUB_IMAGE='\''" ref "'\''" }
  ' "$env_file" >"$tmp"
  chown root:root "$tmp"
  chmod 0600 "$tmp"
  mv "$tmp" "$env_file"
}

make_backup_dir() {
  if [ -L "$backup_root" ] || { [ -e "$backup_root" ] && [ ! -d "$backup_root" ]; }; then
    die "$backup_root is not a directory"
  fi
  # An existing parent keeps its mode: --backup-dir /tmp must not change /tmp.
  [ -d "$backup_root" ] || install -d -m 0700 -o root -g root "$backup_root"
  backup_dir=$backup_root/$stamp
  mkdir -m 0700 "$backup_dir" 2>/dev/null || die "$backup_dir exists or cannot be made; run the script again"
}

container_of() {
  docker ps -q --filter "label=com.docker.compose.project=$project" \
    --filter "label=com.docker.compose.service=$1" --filter status=running | head -n 1
}

psql_value() {
  local cid
  cid=$(container_of postgres)
  [ -n "$cid" ] || return 1
  docker exec "$cid" psql -X -U "$pg_user" -d "$pg_db" -Atqc "$1"
}

schema_version() {
  psql_value 'SELECT coalesce(max(version), 0) FROM _sqlx_migrations WHERE success'
}

data_mount() {
  docker inspect --format "{{range .Mounts}}{{if eq .Destination \"$pg_data\"}}{{.Type}} {{.Name}} {{.Source}}{{end}}{{end}}" "$1"
}

# Records the database of the running stack, so the new stack can prove that it uses the
# same one. $1 is the directory of the running stack.
record_database() {
  local cid label
  cid=$(container_of postgres)
  [ -n "$cid" ] || die "the PostgreSQL container of the project $project does not run; start the stack first"
  label=$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}' "$cid")
  [ "$label" = "$1" ] || die "the running stack of the project $project comes from $label, not from $1"
  label=$(docker inspect --format '{{index .Config.Labels "com.docker.compose.project.config_files"}}' "$cid")
  [ "$label" = "$1/compose.yaml" ] ||
    die "the running stack uses the compose files $label, not only $1/compose.yaml; the new stack can start on another database"
  db_mount_before=$(data_mount "$cid")
  case "$db_mount_before" in
    "volume $pg_volume "*) ;;
    *) die "the database of the running stack is in \"$db_mount_before\", not in the volume $pg_volume; the new stack can start on another database" ;;
  esac
  db_id_before=$(psql_value 'SELECT system_identifier FROM pg_control_system()') ||
    die "the script cannot read the identifier of the database"
  schema_before=$(schema_version) || die "the script cannot read the schema version of the database"
  users_before=$(psql_value 'SELECT count(*) FROM users') || die "the script cannot read the users of the database"
  say "The database: volume $pg_volume, identifier $db_id_before, schema version $schema_before"
}

check_same_database() {
  local cid mount id
  cid=$(container_of postgres)
  mount=$(data_mount "$cid")
  id=$(psql_value 'SELECT system_identifier FROM pg_control_system()' || true)
  if [ "$mount" != "$db_mount_before" ] || [ "$id" != "$db_id_before" ]; then
    db_mismatch=true
    compose down || true
    die "the new stack uses another database (\"$mount\", identifier $id), not \"$db_mount_before\", identifier $db_id_before; the script stopped it"
  fi
}

# Only the hashes leave the container. $1 is the KEK file that the new stack mounts.
check_kek_of_running_cp() {
  local cid running expected
  cid=$(container_of cp)
  [ -n "$cid" ] || die "the control plane container does not run, so the script cannot compare its KEK; start the stack first"
  running=$(docker exec "$cid" cat /run/secrets/restichub_kek | sha256sum | cut -d' ' -f1) ||
    die "the script cannot read the KEK of the running control plane"
  expected=$(sha256sum <"$1" | cut -d' ' -f1)
  [ "$running" = "$expected" ] ||
    die "the KEK of the running control plane is not the file $1 (SHA-256 ${running:0:8}, not ${expected:0:8}); find the KEK that the control plane uses"
}

check_free_space() {
  local size need dir avail docker_root
  size=$(psql_value 'SELECT pg_database_size(current_database())') || die "the script cannot read the size of the database"
  docker_root=$(docker info --format '{{.DockerRootDir}}')
  local dirs=("$backup_root")
  need=$((size * 2))
  if [ ! -d "$docker_root" ]; then
    warn "the Docker data directory $docker_root is not on this host; the script checks only $backup_root"
  elif [ "$(stat -c %d "$backup_root")" = "$(stat -c %d "$docker_root")" ]; then
    need=$((size * 4))
  else
    dirs+=("$docker_root")
  fi
  for dir in "${dirs[@]}"; do
    avail=$(($(df -Pk "$dir" | awk 'NR == 2 { print $4 }') * 1024))
    [ "$avail" -ge "$need" ] ||
      die "$dir has $((avail / 1048576)) MiB free; the database has $((size / 1048576)) MiB, and the run needs $((need / 1048576)) MiB"
  done
}

dump_database() {
  local cid dump=$backup_dir/restichub-$stamp.dump
  cid=$(container_of postgres)
  say "Writing the database dump to $dump"
  # The superuser makes the dump, because the tables have row-level security.
  docker exec "$cid" pg_dump -U "$pg_user" -Fc "$pg_db" >"$dump" || die "pg_dump failed"
  [ -s "$dump" ] || die "the dump $dump is empty"
  # Without a database, pg_restore reads all the data of the dump and writes it nowhere.
  docker exec -i "$cid" pg_restore -f /dev/null <"$dump" || die "pg_restore cannot read the dump $dump"
  dump_file=$dump
}

place_release() {
  local staged
  staged=$(mktemp -d "$(dirname "$install_dir")/.restichub-new.XXXXXX")
  cp -R "$work/release/." "$staged/"
  ln -s "$env_file" "$staged/.env"
  chown -R root:root "$staged"
  chmod 0755 "$staged"
  if [ ! -e "$install_dir" ]; then
    mv "$staged" "$install_dir"
  elif mv --help 2>/dev/null | grep -- '--exchange' >/dev/null; then
    # renameat2 with RENAME_EXCHANGE swaps the two directories in one step.
    mv --exchange -T "$staged" "$install_dir"
    rm -rf "$staged"
  else
    # rename(2) cannot replace a directory that is not empty. Two renames in one parent
    # leave a gap of microseconds, and the old files stay until the new ones are in place.
    local old
    old=$(dirname "$install_dir")/.restichub-old.$$
    mv "$install_dir" "$old"
    mv "$staged" "$install_dir"
    rm -rf "$old"
  fi
  placed=true
}

compose() {
  (cd "$install_dir" && docker compose "$@")
}

# $1 is the service, $2 the number of 5-second waits.
wait_healthy() {
  local cid='' status='' restarts i
  for i in $(seq 1 "$2"); do
    cid=$(compose ps -q "$1")
    if [ -n "$cid" ]; then
      status=$(docker inspect --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$cid")
      [ "$status" = healthy ] && return 0
      restarts=$(docker inspect --format '{{.RestartCount}}' "$cid")
      [ "$restarts" -lt 3 ] ||
        die "the service $1 stopped $restarts times; read the log with: cd $install_dir && docker compose logs migrate $1"
    fi
    [ "$i" -lt "$2" ] || die "the service $1 is not healthy after $(($2 * 5)) seconds; read the log with: cd $install_dir && docker compose logs migrate $1"
    sleep 5
  done
}

start_stack() {
  say "Starting the stack"
  compose pull --quiet
  local started
  started=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  if [ "$mode" != install ]; then
    # Only PostgreSQL starts first, so no migration runs before the database is proven.
    compose up -d postgres ||
      die "PostgreSQL did not start; read the log with: cd $install_dir && docker compose logs postgres"
    wait_healthy postgres 24
    check_same_database
  fi
  compose up -d ||
    die "the stack did not start; read the log with: cd $install_dir && docker compose logs migrate cp"
  wait_healthy cp 60
  local cid running expected
  cid=$(compose ps -q cp)
  running=$(docker inspect --format '{{.Config.Image}}' "$cid")
  [ "$running" = "$image_ref" ] || die "the control plane runs $running, not the verified $image_ref"
  running=$(docker inspect --format '{{.Image}}' "$cid")
  expected=$(docker image inspect --format '{{.Id}}' "$image_ref")
  [ "$running" = "$expected" ] || die "the image of the running control plane is not the verified image"
  if [ "$mode" != install ] && [ "$users_before" -gt 0 ] &&
    compose logs --no-color --since "$started" cp 2>&1 | grep setup_token >/dev/null; then
    die "the control plane printed a new setup token, so its database has no administrator; it does not use the database of the old stack"
  fi
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

# $1 is the directory of the running stack, $2 the KEK file of the new stack.
prepare_switch() {
  record_database "$1"
  check_kek_of_running_cp "$2"
  make_backup_dir
  check_free_space
}

finish_switch() {
  local schema_now
  schema_now=$(schema_version) || schema_now=''
  changing=false
  finished=true
  rm -f "$failed_marker"
  write_rollback_file "$schema_now"
  say "The schema version: $schema_before -> ${schema_now:-unknown}."
}

run_install() {
  check_install_target
  check_ports
  get_cosign
  get_release
  verify_image
  write_config
  # check_install_target found no directory or an empty one.
  record "$install_dir"
  place_release
  start_stack
  created=()
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
  prepare_switch "$install_dir" "$kek_file"
  cp -p "$env_file" "$backup_dir/restichub.env"
  cp -a "$install_dir" "$backup_dir/release"
  say "Saved the environment file and the release files to $backup_dir"
  dump_database
  local caddy_changed=false
  cmp -s "$install_dir/Caddyfile" "$work/release/Caddyfile" || caddy_changed=true
  changing=true
  write_rollback_file ''
  place_release
  set_env_image
  start_stack
  # A file bind mount keeps the inode of the replaced Caddyfile, so Caddy reads the new
  # file only in a new container.
  if $caddy_changed; then
    say "The Caddyfile changed; recreating the caddy service"
    compose up -d --force-recreate --no-deps caddy
  fi
  finish_switch
  say "The upgrade to $version is complete. The backup is in $backup_dir."
}

run_migrate() {
  check_migrate_target
  get_cosign
  get_release
  verify_image
  prepare_switch "$src_dir" "$src_kek"
  copy_config
  dump_database
  git_backup=$git_dir.git-backup-$stamp
  changing=true
  write_rollback_file ''
  mv "$git_dir" "$git_backup"
  say "Moved the git clone to $git_backup"
  place_release
  set_env_image
  start_stack
  finish_switch
  cat <<EOF

The server uses the files of this script now. The KEK is in $kek_file.
Its SHA-256 starts with $(kek_fingerprint), the same as the KEK of the clone.
The old clone is in $git_backup and still holds the KEK and the environment file.
Make sure that a copy of $kek_file and $env_file is outside the server, and that the hosts
are Online. Then delete the old clone: rm -rf $git_backup
EOF
}

main() {
  parse_args "$@"
  trap on_exit EXIT
  check_host
  take_lock
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  make_work_dir
  case "$mode" in
    install) run_install ;;
    upgrade) run_upgrade ;;
    migrate) run_migrate ;;
  esac
}

main "$@"

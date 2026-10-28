---
title: Install the control plane
description: Install the control plane, the web interface and the agent listener on one server with one script.
---

The control plane runs on one Linux server. This page installs the control plane, PostgreSQL, the web interface and the agent listener on that server. The script `install.sh` of the release installs the reference stack with Docker Compose. A large installation needs its own design.

## The parts

| Part | Task |
|---|---|
| Control plane | Keeps the configuration, schedules the jobs, serves the API and the web interface |
| PostgreSQL | Keeps the installation data |
| Reverse proxy (Caddy) | Terminates the TLS of the web interface and gets the certificate |
| Agent listener | Serves the agent protocol on its own port |

## Before you start

- Get a Linux server with at least 2 GB of memory and access to the internet. The server must have the architecture `x86_64` or `arm64`.
- Install Docker Engine and the Compose plugin on the server. Use the [Docker guide](https://docs.docker.com/engine/install/). The server needs no `git`.
- Make sure that `curl`, `sudo`, GNU coreutils and GNU tar are on the server. The script does not run with the BusyBox tools.
- Make sure that `/var/tmp` does not have the mount option `noexec`. The script runs `cosign` from this directory. To use another directory, set `TMPDIR` for the script.
- Open a `tmux` or `screen` session on the server. If the SSH connection stops, the session keeps the script running.
- Get a domain name for the control plane, for example `restichub.example.com`.
- Create a DNS record for the domain. Use the type A and the public address of the server:

  ```text
  restichub.example.com.  A  203.0.113.10
  ```

- Open these ports in the firewall of the server:

| Port | Protocol | Purpose |
|---|---|---|
| 22 | TCP | SSH administration |
| 80 | TCP | The certificate challenge of the reverse proxy |
| 443 | TCP | The web interface, the API and the agent enrollment |
| 8443 | TCP | The agent listener |

The reverse proxy gets a public certificate for the domain at its first start. It uses the ACME protocol. The DNS record must point to the server, and the ports 80 and 443 must be reachable from the internet before you start the script.

## Install

1. Open the [releases page](https://github.com/fgbm/restichub/releases) and find the version of the release, for example `1.2.0`.
2. Set the version in the shell:

   ```sh
   VERSION=<version>
   ```

3. Run the script of the release. Use your domain and your email address:

   ```sh
   curl -fsSL "https://github.com/fgbm/restichub/releases/download/v${VERSION}/install.sh" | sudo bash -s -- --domain restichub.example.com --email admin@example.com
   ```

   The script prints `The control plane is healthy and runs the verified image` and the path of the key encryption key (KEK).

4. Do the steps in [Store the KEK outside the server](#store-the-kek-outside-the-server). The script waits until you type `saved`.

The script does these steps:

1. It checks that it runs as root and that Docker Engine and Docker Compose 2 or later answer. It also checks that the ports 80, 443 and 8443 are free. Only one run of the script at a time can hold the lock `/run/restichub-install.lock`.
2. It stops when the server is not empty. It looks for the KEK file, the environment file and the volume `restichub_pgdata`. It also looks for each volume and container of the project `restichub`. The message names what it found. For an installed server, use `--upgrade`. For a server that was installed with `git clone`, use `--migrate-from-git`.
3. It downloads `cosign` and compares its SHA-256 with a fixed value.
4. It downloads the archive `restichub-deploy-<version>.tar.gz` and verifies its signature. The signature must come from the release workflow at the tag of the version.
5. It reads the image reference from the archive and verifies the signature of the image with the same identity.
6. It writes the environment file and the KEK to `/etc/restichub`, and the release files to `/opt/restichub`.
7. It starts the stack, waits until the control plane is healthy, and compares the running image with the verified image.

If a check fails, the script stops before it changes the server. No option of the script skips a check. If the installation fails after the script wrote files, the script prints the commands that remove only the files and the volumes of this run.

## Install with a verified download

The command in [Install](#install) runs the script before you verify it. It trusts the HTTPS connection to GitHub. To verify the script first, do these steps:

1. Get `cosign` with the step "Get cosign" of [Verify a release](/restichub/operations/verify-a-release/#get-cosign).
2. Set the version of the release:

   ```sh
   VERSION=<version>
   ```

3. Download the script and its signature bundle:

   ```sh
   curl -fsSLO "https://github.com/fgbm/restichub/releases/download/v${VERSION}/install.sh"
   curl -fsSLO "https://github.com/fgbm/restichub/releases/download/v${VERSION}/install.sh.sigstore.json"
   ```

4. Verify the signature of the script:

   ```sh
   cosign verify-blob \
     --bundle install.sh.sigstore.json \
     --certificate-identity "https://github.com/fgbm/restichub/.github/workflows/release.yml@refs/tags/v${VERSION}" \
     --certificate-oidc-issuer https://token.actions.githubusercontent.com \
     install.sh
   ```

   The command prints `Verified OK`.

5. Show the SHA-256 of the script:

   ```sh
   sha256sum install.sh
   ```

6. Compare the value with the line `install.sh` in the section "Install the control plane" of the release notes. Do not run the script when the values differ.
7. Run the script:

   ```sh
   sudo bash install.sh --domain restichub.example.com --email admin@example.com
   ```

For a run without a terminal, for example from an automation tool, add `--kek-saved`. With this option, you confirm that the automation copies the KEK and the environment file outside the server. Without a terminal and without the option, the script refuses to start.

## Store the KEK outside the server

:::danger
Copy the KEK and the environment file to a place outside the server now. Without the KEK, nobody can decrypt the repository passwords and the keys in the database. A backup of the database is not usable without the KEK.
:::

1. In a second terminal on the server, write the directory `/etc/restichub` to an archive that only you can read:

   ```sh
   rm -f ~/restichub-config.tar.gz
   (umask 077 && sudo tar -czf - -C /etc restichub > ~/restichub-config.tar.gz)
   ```

   The first command removes an old archive, because `umask` does not change the mode of a file that exists.

2. On your workstation, copy the archive from the server:

   ```sh
   scp admin@restichub.example.com:restichub-config.tar.gz .
   ```

3. On the server, delete the archive:

   ```sh
   rm ~/restichub-config.tar.gz
   ```

4. On your workstation, show the first 8 characters of the SHA-256 of the KEK in the archive:

   ```sh
   tar -xzOf restichub-config.tar.gz restichub/secrets/restichub_kek | sha256sum | cut -c1-8
   ```

5. Make sure that the value is the same as the value that the script printed.
6. Put the archive in a password manager or a safe. Keep it apart from the backups of the database.
7. Delete the archive from your workstation.
8. In the terminal of the script, type `saved`. After `install.sh --migrate-from-git`, skip this step. The script does not wait for the word, because it uses the KEK of the clone.

## Check the installation

Do the steps in a root shell (`sudo -i`). Only root can read the environment file, so each `docker compose` command needs root.

1. Go to the directory of the stack:

   ```sh
   cd /opt/restichub
   ```

2. Make sure that the control plane is healthy:

   ```sh
   docker compose ps cp
   ```

   The column `STATUS` shows `(healthy)`.

3. Make sure that the reverse proxy has the certificate:

   ```sh
   curl -fsS https://restichub.example.com/healthz
   ```

   The command prints `ok`. If the command shows a TLS error, read the log of the proxy with `docker compose logs caddy`.

4. Read the one-time setup token:

   ```sh
   docker compose logs cp | grep setup_token
   ```

## The files of the installation

| Place | Content | Owner and mode |
|---|---|---|
| `/opt/restichub` | The release files: `compose.yaml`, `Caddyfile`, `bootstrap-roles.sql`, `install.sh` and the image reference `IMAGE`. The link `.env` points to `/etc/restichub/.env`. Each upgrade replaces the directory. | root, `755` |
| `/etc/restichub/.env` | The environment file with the passwords | root, `600` |
| `/etc/restichub/secrets/restichub_kek` | The KEK | uid 10001, `400` |
| Docker volume `restichub_pgdata` | The database | Docker |
| Docker volumes `restichub_caddy_data` and `restichub_caddy_config` | The certificate and the state of Caddy | Docker |

Do not keep your own files in `/opt/restichub`. An upgrade replaces the directory.

## The first administrator

:::danger
The setup token is a secret. Do not send it to a shared terminal or a chat. The control plane consumes the token at the first use, and the token expires after one day.
:::

1. Open `https://restichub.example.com` in a browser. The screen "Initial setup" opens.
2. Enter the token in the field "Setup token".
3. Enter a user name and a password with 12 characters or more.
4. Select "Create the administrator".
5. Sign in with the new user name and password.

## The environment file

The stack reads one file, `/etc/restichub/.env`. Compose finds it through the link `/opt/restichub/.env`. The script `install.sh` writes the file with random secrets. The file mode is `600`. The script does not overwrite an existing file. It writes each value in single quotes, so Compose does not expand a `$` in a value.

| Value | Content |
|---|---|
| `POSTGRES_USER` | The PostgreSQL superuser of the stack |
| `POSTGRES_PASSWORD` | The password of that superuser |
| `POSTGRES_DB` | The name of the installation database |
| `RESTICHUB_OWNER_PASSWORD` | The password of the owner login. The migrations use it |
| `RESTICHUB_RUNTIME_PASSWORD` | The password of the runtime login. The control plane uses it |
| `RESTICHUB_DOMAIN` | The domain name of the control plane |
| `RESTICHUB_ACME_EMAIL` | The email address for the certificate |
| `RESTICHUB_IMAGE` | The image reference with the digest of the release |
| `RESTICHUB_AGENT_ENDPOINT` | The address of the agent listener that the agents dial. The script sets `https://<domain>:8443` |
| `RESTICHUB_TENANCY_MODE` | `single` or `multi`. The script sets `single` |
| `RESTICHUB_TRUSTED_PROXIES` | The network of the reverse proxy in CIDR form |

The optional value `RESTICHUB_AGENT_PORT` changes the published port of the agent listener. The default is 8443. The page [Environment variables](/restichub/reference/environment-variables/) lists every other setting of the control plane.

The KEK is not in the environment file. `install.sh` writes it to the file `/etc/restichub/secrets/restichub_kek` with the mode `400`, and Docker mounts the file as a secret. The control plane reads it through `RESTICHUB_KEK_FILE`. The key opens the repository passwords and the installation CA. The control plane user (uid 10001) must own the file. The control plane refuses a file that others can read.

:::caution
Do not pass the key in the environment variable `RESTICHUB_KEK`. `docker inspect` and the process environment show the value. The variable works for compatibility, but it is not recommended.
:::

## Upgrade

See [Upgrade](/restichub/guides/upgrade/). The script `install.sh --upgrade` makes a backup before it changes the server. A server that was installed with `git clone` moves to the new layout with `install.sh --migrate-from-git`.

## Monitoring

Poll `/readyz` with an external monitor. Use `/healthz` for the container and the load balancer. See [Monitor the control plane](/restichub/operations/monitor-the-control-plane/).

## Backup

Back up the database and the KEK, and keep them apart. See [Back up the control plane](/restichub/operations/back-up-the-control-plane/).

## The agent listener

The control plane publishes the agent listener on its own port. The listener terminates the TLS itself, so the reverse proxy must not terminate the connection.

An agent sends its first enrollment request to the domain on port 443. The reverse proxy serves this request with its public certificate. The agent then receives the installation root CA and uses it for the connection to the agent listener.

:::caution
The agent listener port must be reachable from every agent network. Check the firewall rules of the server and of the network path.
:::

Set `RESTICHUB_AGENT_ENDPOINT` to the address that the agents dial. The control plane issues the listener certificate with that host as a subject alternative name.

Each agent connection uses one file descriptor of the control plane. At start, the control plane sets its soft open file limit to the hard limit and writes the limit to the log. If the limit is too low for `RESTICHUB_AGENT_MAX_CONNECTIONS`, the log has a warning. The `cp` service of the reference compose file sets `ulimits: nofile` to 65536. If you use another runtime, set the same limit (for systemd, `LimitNOFILE=65536`).

Enroll each host as described in [Register an agent](/restichub/getting-started/add-a-host/).

## Related

- [Verify a release](/restichub/operations/verify-a-release/)
- [Register an agent](/restichub/getting-started/add-a-host/)
- [Agent enrollment rate limit](/restichub/operations/enrollment-rate-limit/)
- [Root CA rotation](/restichub/operations/root-ca-rotation/)
- [Install from a release archive](https://github.com/fgbm/restichub/blob/main/design/adr/0056-install-from-a-release-archive.md)
- [Single-tenant mode](https://github.com/fgbm/restichub/blob/main/design/adr/0024-single-tenant-mode.md)
- [Agent PKI identity and key rotation](https://github.com/fgbm/restichub/blob/main/design/adr/0026-agent-pki-identity-and-key-rotation.md)

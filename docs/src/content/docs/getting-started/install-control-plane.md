---
title: Install the control plane
description: Install the control plane, the web interface and the agent listener on one server with Docker Compose.
---

The control plane runs on one Linux server. This page installs the control plane, PostgreSQL, the web interface and the agent listener on that server. The steps use the reference stack in the directory `deploy/reference` of the repository and Docker Compose. A large installation needs its own design.

## The parts

| Part | Task |
|---|---|
| Control plane | Keeps the configuration, schedules the jobs, serves the API and the web interface |
| PostgreSQL | Keeps the installation data |
| Reverse proxy (Caddy) | Terminates the TLS of the web interface and gets the certificate |
| Agent listener | Serves the agent protocol on its own port |

## Before you start

- Get a Linux server with at least 2 GB of memory and access to the internet.
- Install Docker Engine, the Compose plugin and `git` on the server.
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

The reverse proxy gets a public certificate for the domain at its first start. It uses the ACME protocol. The DNS record must point to the server, and the ports 80 and 443 must be reachable from the internet before you start the stack.

## Get the image digest

Install the image of a release by its digest. A tag can move, and a digest cannot.

1. Open the [releases page](https://github.com/fgbm/restichub/releases) and select the release.
2. Find the image reference in the section "Contents". The reference has the form `ghcr.io/fgbm/restichub-cp@sha256:<digest>`.
3. Verify the image with the steps in [Verify a release](/restichub/operations/verify-a-release/).

## Install

Do the steps in a root shell (`sudo -i`). The environment file and the key file are readable only by their owner, and the control plane user (uid 10001) must own the key file.

1. Check out the release tag into `/opt/restichub`. Use the tag of the release, for example `v0.1.0-rc.4`:

   ```sh
   git clone --depth 1 --branch v0.1.0-rc.4 https://github.com/fgbm/restichub.git /opt/restichub
   ```

2. Go to the directory of the reference stack:

   ```sh
   cd /opt/restichub/deploy/reference
   ```

3. Create the environment file. Use your domain, your email address and the image reference with the digest:

   ```sh
   ./init-env.sh restichub.example.com admin@example.com ghcr.io/fgbm/restichub-cp@sha256:<digest>
   ```

   The script prints `wrote .env`.

4. Start the stack:

   ```sh
   docker compose up -d
   ```

5. Make sure that the control plane is healthy:

   ```sh
   docker compose ps cp
   ```

   The column `STATUS` shows `(healthy)`. The `migrate` service applies the database migrations before the control plane starts, so the first start can take a minute.

6. Make sure that the reverse proxy has the certificate:

   ```sh
   curl -fsS https://restichub.example.com/healthz
   ```

   The command prints `ok`. If the command shows a TLS error, read the log of the proxy with `docker compose logs caddy`.

7. Read the one-time setup token:

   ```sh
   docker compose logs cp | grep setup_token
   ```

The stack keeps the compose file in a checkout of the release tag. The stack needs other files of the repository too: the service `db-bootstrap` reads `crates/restichub-db/sql/bootstrap-roles.sql`. Do not copy the directory `deploy/reference` alone.

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

The stack reads one file, `.env`, next to `compose.yaml`. The script `init-env.sh` writes the file with random secrets. The file mode is `600`. The script does not overwrite an existing file.

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

The key encryption key is not in `.env`. `init-env.sh` writes it to the file `secrets/restichub_kek` with the mode `400`, and Docker mounts the file as a secret. The control plane reads it through `RESTICHUB_KEK_FILE`. The key opens the repository passwords and the installation CA. The control plane user (uid 10001) must own the file. The control plane refuses a file that others can read. If you run `init-env.sh` as a user other than root, run `sudo chown 10001:10001 secrets/restichub_kek` after it.

:::caution
Do not pass the key in the environment variable `RESTICHUB_KEK`. `docker inspect` and the process environment show the value. The variable works for compatibility, but it is not recommended.
:::

## Upgrade

See [Upgrade](/restichub/guides/upgrade/). Make a backup before each upgrade.

## Backup

Back up the database and the key encryption key, and keep them apart. See [Back up the control plane](/restichub/operations/back-up-the-control-plane/).

## The agent listener

The control plane publishes the agent listener on its own port. The listener terminates the TLS itself, so the reverse proxy must not terminate the connection.

An agent sends its first enrollment request to the domain on port 443. The reverse proxy serves this request with its public certificate. The agent then receives the installation root CA and uses it for the connection to the agent listener.

:::caution
The agent listener port must be reachable from every agent network. Check the firewall rules of the server and of the network path.
:::

Set `RESTICHUB_AGENT_ENDPOINT` to the address that the agents dial. The control plane issues the listener certificate with that host as a subject alternative name.

Enroll each host as described in [Register an agent](/restichub/getting-started/add-a-host/).

## Related

- [Verify a release](/restichub/operations/verify-a-release/)
- [Register an agent](/restichub/getting-started/add-a-host/)
- [Agent enrollment rate limit](/restichub/operations/enrollment-rate-limit/)
- [Root CA rotation](/restichub/operations/root-ca-rotation/)
- [Single-tenant mode](https://github.com/fgbm/restichub/blob/main/design/adr/0024-single-tenant-mode.md)
- [Agent PKI identity and key rotation](https://github.com/fgbm/restichub/blob/main/design/adr/0026-agent-pki-identity-and-key-rotation.md)

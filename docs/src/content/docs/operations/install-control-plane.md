---
title: Install the control plane
description: Install the control plane, the web interface and the agent listener on one server with Docker Compose.
---

The control plane runs on one Linux server. This page installs the control plane, PostgreSQL, the web interface and the agent listener on that server. The steps use Docker Compose. A large installation needs its own design.

## The parts

| Part | Task |
|---|---|
| Control plane | Keeps the configuration, schedules the jobs, serves the API and the web interface |
| PostgreSQL | Keeps the installation data |
| Reverse proxy | Terminates the TLS of the web interface and gets the certificate |
| Agent listener | Serves the agent protocol on its own port |

The repository holds a reference stack in `deploy/stage`. Copy the files from that directory into `/opt/restichub` on the server. Change every value for your installation.

## Before you start

- Install Docker Engine and the Compose plugin on the server.
- Give the server at least 2 GB of memory. The reference stack fits in this size.
- Create a DNS record for the control plane host. Use the type A and the address of the server:

  ```text
  stage.example.com.  A  203.0.113.10
  ```

- Open these ports in the firewall of the server:

| Port | Protocol | Purpose |
|---|---|---|
| 22 | TCP | SSH administration |
| 80 | TCP | The certificate challenge of the reverse proxy |
| 443 | TCP | The web interface and the API |
| 8443 | TCP | The agent listener |

## The environment file

The stack reads one file, `.env`, next to `compose.yaml`. The script `bootstrap.sh` writes the file with random secrets. The file mode is `600`.

| Value | Content |
|---|---|
| `POSTGRES_USER` | The PostgreSQL superuser of the stack |
| `POSTGRES_PASSWORD` | The password of that superuser |
| `POSTGRES_DB` | The name of the installation database |
| `RESTICHUB_OWNER_PASSWORD` | The password of the owner login. The migrations use it |
| `RESTICHUB_RUNTIME_PASSWORD` | The password of the runtime login. The control plane uses it |
| `RESTICHUB_DOMAIN` | The host name of the control plane |
| `RESTICHUB_ACME_EMAIL` | The email address for the certificate |
| `RESTICHUB_IMAGE_TAG` | The image tag. `edge` follows the newest commit, `sha-<short>` pins one commit |

The key encryption key is not in `.env`. `bootstrap.sh` writes it to the file `secrets/restichub_kek` with the mode `400`, and Docker mounts the file as a secret. The control plane reads it through `RESTICHUB_KEK_FILE`. The key opens the repository passwords and the installation CA. The control plane user (uid 10001) must own the file, because the control plane refuses a file that others can read. Run `bootstrap.sh` as root, or run `sudo chown 10001:10001 secrets/restichub_kek` after it.

:::caution
Do not pass the key in the environment variable `RESTICHUB_KEK`. `docker inspect` and the process environment show the value. The variable works for compatibility, but it is not recommended.
:::

The table below lists the other values.
| `RESTICHUB_TENANCY_MODE` | `single` or `multi` |
| `RESTICHUB_TRUSTED_PROXIES` | The network of the reverse proxy in CIDR form |
| `RESTICHUB_AGENT_ENDPOINT` | The public address of the agent listener that an agent reports |
| `RESTICHUB_AGENT_PORT` | The published port of the agent listener. The default is 8443 |

The page [Environment variables](/restichub/operations/environment-variables/) lists every other setting of the control plane.

The value `RESTICHUB_AGENT_ENDPOINT` holds the address that the agents dial, for example `https://agents.example.com:8443`. Set the host and the port of your installation.

## Install

1. Copy the files into `/opt/restichub` on the server.
2. Create the environment file:

   ```sh
   cd /opt/restichub
   ./bootstrap.sh .env stage.example.com admin@example.com
   ```

3. Pull the image and start the stack:

   ```sh
   docker compose up -d
   ```

4. Wait until the control plane answers `/healthz`. The `migrate` service applies the database migrations before the control plane starts.
5. Read the one-time setup token:

   ```sh
   docker compose logs cp | grep setup_token
   ```

## The first administrator

Open `https://stage.example.com` in a browser. The screen asks for the setup token. Enter the token, then create the first administrator with a user name and a password.

:::danger
The setup token is a secret. Do not send it to a shared terminal or a chat. The control plane consumes the token at the first use, and the token expires after one day.
:::

## Upgrade

1. Set `RESTICHUB_IMAGE_TAG` in `.env` to the new tag.
2. Pull the image and restart the stack:

   ```sh
   docker compose pull
   docker compose up -d
   ```

The `migrate` service applies the new migrations before the control plane starts.

## Backup

The control plane keeps the repository passwords in the database, wrapped with the key encryption key. The installation CA is in the database too, so one database copy holds the whole installation.

1. Back up the database:

   ```sh
   docker compose exec -T postgres pg_dump -U restichub_owner restichub > restichub.sql
   ```

2. Copy the file to another host.
3. Keep the key encryption key with the copy. The key is in the file `secrets/restichub_kek`.

:::danger
Losing the database or the key encryption key loses the repository passwords and the installation CA. Restic cannot open a repository without its password. Keep the copies on another host.
:::

## The agent listener

The control plane publishes the agent listener on its own port. The listener terminates the TLS itself, so the reverse proxy must not terminate the connection.

:::caution
The agent listener port must be reachable from every agent network. Check the firewall rules of the server and of the network path.
:::

Set `RESTICHUB_AGENT_ENDPOINT` to the address that the agents dial. The control plane issues the listener certificate with that host as a subject alternative name.

Enroll each host as described in [Register an agent](register-an-agent.md).

## Related

- [Register an agent](register-an-agent.md)
- [Agent enrollment rate limit](enrollment-rate-limit.md)
- [Root CA rotation](root-ca-rotation.md)
- [Single-tenant mode](https://github.com/fgbm/restichub/blob/main/design/adr/0024-single-tenant-mode.md)
- [Agent PKI identity and key rotation](https://github.com/fgbm/restichub/blob/main/design/adr/0026-agent-pki-identity-and-key-rotation.md)

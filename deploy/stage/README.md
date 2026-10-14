# The staging environment

This directory holds the staging deployment of ResticHub. The staging server is one host with 2 GB of memory. Caddy is the only service that publishes a port.

## Files

| File | Contents |
|---|---|
| `compose.yaml` | The services of the staging stack |
| `Caddyfile` | The reverse proxy and the TLS certificate |
| `.env.example` | The variables of the stack. The file holds no secret |
| `bootstrap.sh` | Creates `.env` with random secrets on the server |
| `deploy.sh` | Copies the files and starts the stack from the workstation |

## Services

The stack has four long-running parts and two one-shot jobs.

| Service | Task |
|---|---|
| `postgres` | Keeps the data in the named volume `pgdata`. The service is not published |
| `db-bootstrap` | Creates the cluster roles and sets the installation passwords. It runs once |
| `migrate` | Applies the database migrations. It runs once |
| `cp` | Serves the control plane, the web UI and the agent listener. Port 8080 is not published; the agent listener is |
| `caddy` | Publishes port 80 and port 443. It gets the certificate for the domain |

The control plane trusts the `X-Forwarded-For` header from the Caddy network alone. The network is `172.28.0.0/24`. No other peer can set the client address.

## Prepare the server

1. Install Docker Engine and the Compose plugin. The server uses the official Docker repository.
2. Allow port 80, port 443 and the agent listener port (8443 by default) in `ufw`. Keep port 22 open.
3. Add a DNS record for the domain. Use the type `A` and the address `203.0.113.10`:

   ```text
   stage.example.com.  A  203.0.113.10
   ```

## The workstation file

`deploy.sh` reads the host, the domain and the certificate email from the environment. A file `deploy/stage/stage.local.env` can set them. The file is not in the repository. It holds three lines:

```sh
RESTICHUB_STAGE_HOST=<stage-host>
RESTICHUB_DOMAIN=stage.example.com
RESTICHUB_ACME_EMAIL=admin@example.com
```

The host is the SSH alias of the staging server. The deploy script fails if the host is unset. The domain and the email are necessary for the first run alone.

## Deploy

1. Run the deploy script from the workstation:

   ```sh
   deploy/stage/deploy.sh
   ```

   The script copies the files to `/opt/restichub`. It creates `.env` if the file is missing. It pulls the image and starts the stack. It waits for the control plane.

2. Read the one-time setup token on the server:

   ```sh
   ssh <stage-host> 'docker compose -f /opt/restichub/compose.yaml logs cp | grep setup_token'
   ```

   The token is a secret. Do not print it to a shared terminal.

3. Open `https://stage.example.com` in a browser. Use the token to create the Platform Admin.

## The environment file

`bootstrap.sh` writes `.env` with random secrets and the mode `600`. The file holds the PostgreSQL passwords and the key encryption key. The deploy script never copies the file from the workstation. To make a new file, remove the old one and run `bootstrap.sh` again. A new key encryption key makes the stored secrets unreadable.

## The image tag

The stack uses the tag `edge` by default. Set `RESTICHUB_IMAGE_TAG` in `.env` to a tag `sha-<short>` to deploy one commit. The workflow `Images` publishes both tags.

## The tenancy mode

The staging stack uses the mode `multi`. The mode exercises the tenant path of the product. Set `RESTICHUB_TENANCY_MODE=single` in `.env` to change the mode. See [ADR-0024](../../design/adr/0024-single-tenant-mode.md).

## Stop the stack

```sh
ssh <stage-host> 'docker compose -f /opt/restichub/compose.yaml down'
```

Add the option `-v` to remove the data volumes. The command deletes the PostgreSQL data and the TLS certificates.

## The agent listener

The control plane serves the agent listener on its own port (ADR-0026). It is the gRPC stream with mTLS and the enrollment endpoint `POST /agent/v1/enroll` on one port. The listener terminates TLS itself, so the port is published directly on the `cp` service and Caddy does not proxy it: a proxy would terminate the mTLS of the agents.

Set the public address in `.env` so that an enrolled agent learns where to dial:

```sh
RESTICHUB_AGENT_ENDPOINT=https://agents.example.com:8443
```

The host is the placeholder `agents.example.com` by default. Set the real host and the published port. `RESTICHUB_AGENT_PORT` changes the published port (the default is `8443`). The control plane issues the listener certificate from the installation CA with the endpoint host and the listen address as the subject alternative names.

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
| `cp` | Serves the control plane and the web UI. The service is not published |
| `caddy` | Publishes port 80 and port 443. It gets the certificate for the domain |

The control plane trusts the `X-Forwarded-For` header from the Caddy network alone. The network is `172.28.0.0/24`. No other peer can set the client address.

## Prepare the server

1. Install Docker Engine and the Compose plugin. The server uses the official Docker repository.
2. Allow port 80 and port 443 in `ufw`. Keep port 22 open.
3. Add a DNS record for the domain. Use the type `A` and the address `201.34.151.146`:

   ```text
   rh-stage.app.galen-is.ru.  A  201.34.151.146
   ```

## Deploy

1. Run the deploy script from the workstation:

   ```sh
   deploy/stage/deploy.sh
   ```

   The script copies the files to `/opt/restichub`. It creates `.env` if the file is missing. It pulls the image and starts the stack. It waits for the control plane.

2. Read the one-time setup token on the server:

   ```sh
   ssh restichub-cp-spb-1 'docker compose -f /opt/restichub/compose.yaml logs cp | grep setup_token'
   ```

   The token is a secret. Do not print it to a shared terminal.

3. Open `https://rh-stage.app.galen-is.ru` in a browser. Use the token to create the Platform Admin.

## The environment file

`bootstrap.sh` writes `.env` with random secrets and the mode `600`. The file holds the PostgreSQL passwords and the key encryption key. The deploy script never copies the file from the workstation. To make a new file, remove the old one and run `bootstrap.sh` again. A new key encryption key makes the stored secrets unreadable.

## The image tag

The stack uses the tag `edge` by default. Set `RESTICHUB_IMAGE_TAG` in `.env` to a tag `sha-<short>` to deploy one commit. The workflow `Images` publishes both tags.

## The tenancy mode

The staging stack uses the mode `multi`. The mode exercises the tenant path of the product. Set `RESTICHUB_TENANCY_MODE=single` in `.env` to change the mode. See [ADR-0024](../../design/adr/0024-single-tenant-mode.md).

## Stop the stack

```sh
ssh restichub-cp-spb-1 'docker compose -f /opt/restichub/compose.yaml down'
```

Add the option `-v` to remove the data volumes. The command deletes the PostgreSQL data and the TLS certificates.

## The agent listener

The agent listener (gRPC with mTLS) is not part of this deployment yet. Its port stays closed until epic #1 lands. The port is a comment in `compose.yaml`.

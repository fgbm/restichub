# The reference deployment

This directory holds a neutral Compose stack for one host: PostgreSQL, the control plane and Caddy. It holds no host name, address or secret of any real environment.

Each release publishes these files as the signed archive `restichub-deploy-<version>.tar.gz`, together with `install.sh`. The archive holds `bootstrap-roles.sql` as a file and the file `IMAGE` with the verified image reference. In the repository, `bootstrap-roles.sql` is a link to `crates/restichub-db/sql/bootstrap-roles.sql`. See [ADR-0056](../../design/adr/0056-install-from-a-release-archive.md).

## Install the stack

Do not clone the repository on the server. Run the script of the release as root:

```sh
VERSION=<version>
curl -fsSL "https://github.com/fgbm/restichub/releases/download/v${VERSION}/install.sh" | sudo bash -s -- --domain restichub.example.com --email admin@example.com
```

The script verifies the release files and the image, writes the secrets and starts the stack. The guide [Install the control plane](../../docs/src/content/docs/getting-started/install-control-plane.md) gives all the steps and the verified download of the script.

## The files on the server

| Place | Content |
|---|---|
| `/opt/restichub` | The files of the release archive. Each upgrade replaces them. The link `.env` points to `/etc/restichub/.env`. |
| `/etc/restichub/.env` | The environment file with the passwords, mode `600` |
| `/etc/restichub/secrets/restichub_kek` | The key encryption key, mode `400`, owner uid 10001 |
| Docker volumes | The database (`restichub_pgdata`) and the state of Caddy |

Keep a copy of the environment file and of the key file outside the server. A new key makes the stored secrets unreadable. Docker mounts the key file as a secret, so `docker inspect` does not show the key.

To run the `db-bootstrap` service by hand, add `-T`: `docker compose run -T --rm db-bootstrap`. The option turns off the terminal of the container. The service sets the passwords from the environment file, never from a prompt, and also works with a terminal.

## An installation from git

An installation made with `git clone` holds the repository in `/opt/restichub` and keeps the environment file and the key in `deploy/reference`. Move it to the new layout with `install.sh --migrate-from-git`. See the guide [Upgrade](../../docs/src/content/docs/guides/upgrade.md).

`init-env.sh` is removed. `install.sh` writes the environment file and the key.

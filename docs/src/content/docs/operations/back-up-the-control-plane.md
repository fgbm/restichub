---
title: Back up the control plane
description: Back up the database and the key encryption key of the control plane, and restore them on a new host.
---

The database holds the whole installation: the configuration, the jobs, the audit log, the installation CA and the job signing keys. The repository passwords and the keys are in the database, wrapped with the key encryption key (KEK). The KEK is a file outside the database.

You need three items to restore the control plane:

| Item | Where it is | How to keep it |
|---|---|---|
| The database dump | You make it with `pg_dump` | On another host, for example in a backup storage |
| The KEK | `deploy/reference/secrets/restichub_kek` | In a password manager or a safe, not with the dump |
| The environment file | `deploy/reference/.env` | With the KEK |

:::danger
Keep the KEK apart from the database dump. If an attacker gets the dump and the KEK together, the attacker can read all repository passwords. If you lose the KEK, the dump is not usable, and restic cannot open a repository without its password.
:::

## Back up the database

The tables of the control plane have row-level security. Make the dump as the PostgreSQL superuser of the stack. The value of `POSTGRES_USER` in the environment file gives its name, `restichub` by default. A dump as `restichub_owner` fails.

1. Go to the directory of the reference stack:

   ```sh
   cd /opt/restichub/deploy/reference
   ```

2. Make the dump:

   ```sh
   docker compose exec -T postgres pg_dump -U restichub -Fc restichub > restichub-$(date +%F).dump
   ```

3. Make sure that the command printed no error and that the file is not empty.
4. Copy the file to another host.

Make a dump before each upgrade, and at a regular interval, for example once a day with `cron`. The dump does not stop the control plane.

## Back up the KEK and the environment file

Make this copy one time, after the installation. The KEK does not change.

1. Copy the file `secrets/restichub_kek` to a password manager or a safe.
2. Copy the environment file `.env` to the same place.

## Restore on a new host

Use the same release as the dump. To restore to a newer release, restore first and upgrade after it.

1. Install Docker and check out the release tag, as in [Install the control plane](/restichub/getting-started/install-control-plane/).
2. Copy the environment file to `deploy/reference/.env`.
3. Copy the KEK to `deploy/reference/secrets/restichub_kek`.
4. Give the KEK file to the control plane user:

   ```sh
   sudo chown 10001:10001 secrets/restichub_kek && sudo chmod 400 secrets/restichub_kek
   ```

5. Start the database:

   ```sh
   docker compose up -d postgres
   ```

6. Create the roles of the control plane:

   ```sh
   docker compose run --rm -T db-bootstrap
   ```

7. Restore the dump:

   ```sh
   docker compose exec -T postgres pg_restore -U restichub -d restichub --exit-on-error < restichub.dump
   ```

8. Start the stack:

   ```sh
   docker compose up -d
   ```

9. Wait until `docker compose ps cp` shows `(healthy)`.

If the new host has another address, point the DNS record of the domain to it. The agents connect to the domain, so they find the new host. If the domain changes, register each host again. See [Enroll a host again](/restichub/guides/agent-certificates/).

Check the result:

- You sign in with the same accounts.
- The screen "Hosts" shows the hosts "Online" again.
- A job of a plan succeeds. This proves that the control plane can read the repository passwords with the KEK.

## Related

- [Install the control plane](/restichub/getting-started/install-control-plane/)
- [Upgrade](/restichub/guides/upgrade/)
- [Security for operators](/restichub/operations/security-for-operators/)

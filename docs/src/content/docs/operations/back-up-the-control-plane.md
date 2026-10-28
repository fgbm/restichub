---
title: Back up the control plane
description: Back up the database and the key encryption key of the control plane, and restore them on a new host.
---

The database holds the whole installation: the configuration, the jobs, the audit log, the installation CA and the job signing keys. The repository passwords and the keys are in the database, wrapped with the key encryption key (KEK). The KEK is a file outside the database.

You need three items to restore the control plane:

| Item | Where it is | How to keep it |
|---|---|---|
| The database dump | You make it with `pg_dump` | On another host, for example in a backup storage |
| The KEK | `/etc/restichub/secrets/restichub_kek` | In a password manager or a safe, not with the dump |
| The environment file | `/etc/restichub/.env` | With the KEK |

A server that was installed with `git clone` keeps the KEK and the environment file in `/opt/restichub/deploy/reference`. Move such a server to the new layout with `install.sh --migrate-from-git`. See [Move a server that was installed with git](/restichub/guides/upgrade/#move-a-server-that-was-installed-with-git).

:::danger
Keep the KEK apart from the database dump. If an attacker gets the dump and the KEK together, the attacker can read all repository passwords. If you lose the KEK, the dump is not usable, and restic cannot open a repository without its password.
:::

## Back up the database

The tables of the control plane have row-level security. Make the dump as the PostgreSQL superuser of the stack. The value of `POSTGRES_USER` in the environment file gives its name, `restichub` by default. A dump as `restichub_owner` fails.

1. Open a root shell (`sudo -i`), and go to the directory of the stack:

   ```sh
   cd /opt/restichub
   ```

2. Make the dump:

   ```sh
   docker compose exec -T postgres pg_dump -U restichub -Fc restichub > restichub-$(date +%F).dump
   ```

3. Make sure that the command printed no error and that the file is not empty.
4. Copy the file to another host.

Make a dump before each upgrade, and at a regular interval, for example once a day with `cron`. The dump does not stop the control plane. `install.sh --upgrade` makes a dump by itself before it changes the server.

## Back up the KEK and the environment file

Make this copy one time, after the installation. The KEK does not change. The steps are in [Store the KEK outside the server](/restichub/getting-started/install-control-plane/#store-the-kek-outside-the-server). They give the archive `restichub-config.tar.gz` with the directory `/etc/restichub`.

## Restore on a new host

Use the same release as the dump. To restore to a newer release, restore first and upgrade after it.

1. Install Docker Engine and the Compose plugin, as in [Install the control plane](/restichub/getting-started/install-control-plane/#before-you-start).
2. Copy the archive `restichub-config.tar.gz` and the dump to the new host.
3. Open a root shell (`sudo -i`) in the directory of the two files.
4. Put back the directory `/etc/restichub`. The command keeps the owner and the mode of each file:

   ```sh
   tar -xzpf restichub-config.tar.gz -C /etc
   ```

5. Download and verify the deploy archive `restichub-deploy-<version>.tar.gz` of the release. See [Verify a release](/restichub/operations/verify-a-release/).
6. Unpack the release files to `/opt/restichub`:

   ```sh
   mkdir /opt/restichub && tar -xzf restichub-deploy-<version>.tar.gz -C /opt/restichub --strip-components=1 --no-same-owner
   ```

7. Link the environment file:

   ```sh
   ln -s /etc/restichub/.env /opt/restichub/.env
   ```

8. Go to the directory of the stack:

   ```sh
   cd /opt/restichub
   ```

9. Start the database:

   ```sh
   docker compose up -d postgres
   ```

10. Create the roles of the control plane:

    ```sh
    docker compose run --rm -T db-bootstrap
    ```

11. Restore the dump. Use the path of the dump:

    ```sh
    docker compose exec -T postgres pg_restore -U restichub -d restichub --exit-on-error < /root/restichub.dump
    ```

12. Start the stack:

    ```sh
    docker compose up -d
    ```

13. Wait until `docker compose ps cp` shows `(healthy)`.

If the new host has another address, point the DNS record of the domain to it. The agents connect to the domain, so they find the new host. If the domain changes, register each host again. See [Enroll a host again](/restichub/guides/agent-certificates/).

Check the result:

- You sign in with the same accounts.
- The screen "Hosts" shows the hosts "Online" again.
- A job of a plan succeeds. This proves that the control plane can read the repository passwords with the KEK.

## Related

- [Install the control plane](/restichub/getting-started/install-control-plane/)
- [Upgrade](/restichub/guides/upgrade/)
- [Security for operators](/restichub/operations/security-for-operators/)

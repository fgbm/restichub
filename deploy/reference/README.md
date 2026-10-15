# The reference deployment

This directory holds a neutral Compose stack for one host: PostgreSQL, the control plane and Caddy. Use it as a start for your own installation. It holds no host name, address or secret of any real environment.

## Use the stack

1. Check out the release tag, and [verify the image](../../docs/src/content/docs/operations/verify-a-release.md). The guide shows how to read the digest of the image.
2. Point a DNS record to the host. Open port 80, port 443 and port 8443.
3. Create the environment file:

   ```sh
   cd deploy/reference
   ./init-env.sh restichub.example.com admin@example.com ghcr.io/fgbm/restichub-cp@sha256:<digest>
   ```

4. Start the stack:

   ```sh
   docker compose up -d
   docker compose logs cp | grep setup_token
   ```

5. Open `https://restichub.example.com` and use the setup token to create the first administrator.

To run the `db-bootstrap` service by hand, add `-T`: `docker compose run -T --rm db-bootstrap`. The option turns off the terminal of the container. The service sets the passwords from the environment file, never from a prompt, and also works with a terminal.

Back up the environment file and the file `secrets/restichub_kek`. A new key makes the stored secrets unreadable. The script writes the key file with the mode `400`. The control plane user (uid 10001) must own it: run the script as root, or run `sudo chown 10001:10001 secrets/restichub_kek` after it. Docker mounts the file as a secret, so `docker inspect` does not show the key.

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

Back up the environment file, in particular `RESTICHUB_KEK`. A new key makes the stored secrets unreadable.
